//! Passwordless email-OTP login.
//!
//! - `POST /api/realms/:realm/auth/otp/request` body `{ email }`
//!   Anonymous. Always returns 202 with the same generic message
//!   regardless of whether the email maps to an existing user — same
//!   enumeration-resistance posture as `password-reset/request`.
//!   On a syntactically valid email, a fresh 6-digit code is issued
//!   (invalidating any prior pending one) and mailed.
//!
//! - `POST /api/realms/:realm/auth/otp/login` body `{ email, code }`
//!   Anonymous. Atomically consumes the code:
//!     * Right code → find-or-create user, mark verified=true (the
//!       OTP delivery proved control of the address), issue access +
//!       refresh tokens.
//!     * Wrong code → 401 with `attempts_left` in the body so a
//!       client can show "3 tries remaining".
//!     * Expired / locked / unknown email → 410.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rand_core::{OsRng, RngCore};
use rustbase_auth::{TokenRole, build_claims, encode_token};
use rustbase_core::{CoreError, EmailMessage, RealmId};
use rustbase_db::{
    email_otps::{self, ConsumeOutcome},
    realms::find_realm,
    tokens::{SubjectKind, insert_refresh_token},
    users::{User, find_user_by_email, insert_passwordless_user, mark_verified, record_last_login},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use validator::Validate;

use crate::auth::login::UserPublic;
use crate::auth::{default_access_ttl, default_refresh_ttl, new_refresh_token};
use crate::error::ApiError;
use crate::state::AppState;

const OTP_TTL_MINUTES: i64 = 10;
const SYSTEM_FROM_ADDRESS: &str = "no-reply@rustbase.local";

#[derive(Debug, Deserialize, Validate)]
pub struct OtpRequest {
    #[validate(email)]
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct OtpRequestResponse {
    pub message: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct OtpLoginRequest {
    #[validate(email)]
    pub email: String,
    /// The 6-digit code from the email. Stored + matched as a string
    /// so leading zeros are preserved (the spec output is `000123`,
    /// not `123`).
    #[validate(length(min = 4, max = 12))]
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct OtpLoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserPublic,
}

#[derive(Debug, Serialize)]
pub struct OtpLoginError {
    pub code: &'static str,
    pub attempts_left: Option<i64>,
}

/// Anonymous code-request endpoint. Always 202 — no enumeration signal.
pub async fn request(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<OtpRequest>,
) -> Result<(StatusCode, Json<OtpRequestResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    find_realm(state.system.pool(), &realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.clone())))?;

    let pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;

    let code = fresh_otp_code();
    email_otps::issue(
        &pool,
        &code,
        &req.email,
        chrono::Duration::minutes(OTP_TTL_MINUTES),
    )
    .await?;

    // Mail it. A transport failure is logged but does not change the
    // response shape — the address may not even exist, and either way
    // we keep the enumeration signal flat.
    let body = format!(
        "Hello,\n\nYour one-time login code is:\n\n  {code}\n\n\
         This code is valid for {OTP_TTL_MINUTES} minutes. If you didn't \
         request it, you can safely ignore this email.\n",
    );
    let msg = EmailMessage::new(
        SYSTEM_FROM_ADDRESS,
        &req.email,
        format!("Your login code for realm {realm}"),
        body,
    );
    if let Err(e) = state.mailer.send(msg).await {
        tracing::error!(error = %e, realm = %realm, email = %req.email, "mailer dropped OTP");
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(OtpRequestResponse {
            message: "if that email is valid, a login code has been sent".into(),
        }),
    ))
}

/// Code-redemption endpoint. Issues login tokens on success.
pub async fn login(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<OtpLoginRequest>,
) -> Result<Json<OtpLoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    find_realm(state.system.pool(), &realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.clone())))?;
    let pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;

    match email_otps::consume(&pool, &req.email, &req.code).await? {
        ConsumeOutcome::Ok { email } => issue_tokens_for(&state, &pool, &realm, &email).await,
        ConsumeOutcome::WrongCode { attempts_left } => Err(ApiError::Core(CoreError::Validation(
            format!("wrong code ({attempts_left} attempts left)"),
        ))),
        ConsumeOutcome::Unknown => Err(ApiError::Core(CoreError::Conflict(
            "no pending code for this email — request a new one".into(),
        ))),
        ConsumeOutcome::Expired => Err(ApiError::Core(CoreError::Conflict(
            "code expired — request a new one".into(),
        ))),
        ConsumeOutcome::Locked => Err(ApiError::Core(CoreError::Conflict(
            "too many wrong attempts — request a new code".into(),
        ))),
    }
}

/// Successful-consume path: find-or-create the user, mark verified
/// (the OTP proves email ownership), record the login, mint tokens.
async fn issue_tokens_for(
    state: &AppState,
    pool: &SqlitePool,
    realm: &str,
    email: &str,
) -> Result<Json<OtpLoginResponse>, ApiError> {
    let user: User = match find_user_by_email(pool, email).await? {
        Some(u) => {
            if !u.verified {
                mark_verified(pool, &u.id).await?;
            }
            u
        }
        None => {
            // Passwordless signup. password_hash stays NULL — this
            // user can only ever log in via OTP (or, later, OAuth)
            // until they explicitly set one.
            let fresh = insert_passwordless_user(pool, email).await?;
            mark_verified(pool, &fresh.id).await?;
            tracing::info!(
                realm = %realm,
                user_id = %fresh.id,
                email = %email,
                "user signed up via email OTP"
            );
            fresh
        }
    };

    record_last_login(pool, &user.id).await?;

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(realm.to_string()),
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;
    let refresh = insert_refresh_token(
        pool,
        &new_refresh_token(),
        SubjectKind::User,
        &user.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(realm = %realm, user_id = %user.id, "user login via email OTP");

    Ok(Json(OtpLoginResponse {
        access_token,
        refresh_token: refresh.token,
        user: UserPublic {
            id: user.id,
            email: user.email,
            // mark_verified ran above; refresh the flag on the response
            // so the client doesn't see a stale `false`.
            verified: true,
        },
    }))
}

/// Six random digits. OsRng → u32 → string with leading zeros so
/// `000123` is a valid code (and stored as such).
fn fresh_otp_code() -> String {
    let n = OsRng.next_u32() % 1_000_000;
    format!("{n:06}")
}
