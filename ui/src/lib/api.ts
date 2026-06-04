/**
 * Minimal fetch wrapper for the RustBase REST API.
 *
 * - Same-origin in production (the SvelteKit bundle is served by the
 *   Rust binary at `/_/`, the API at `/api`, `/_/setup`, `/_/auth`).
 * - Same-origin in dev too — the Vite proxy in `vite.config.ts`
 *   forwards those paths to `localhost:8080`.
 *
 * Authentication rides on **`HttpOnly` session cookies** (`rb_at` for
 * access, `rb_rt` for refresh) issued by the login + refresh
 * endpoints. Every fetch sets `credentials: 'include'` so the browser
 * attaches them automatically — no JS-readable token state, no XSS
 * exfiltration surface. Non-2xx responses become a thrown `ApiError`
 * so callers don't have to repeat the same `.ok` / `.json` dance.
 */

export class ApiError extends Error {
	constructor(
		public status: number,
		public code: string,
		message: string,
		public body?: unknown
	) {
		super(message);
		this.name = 'ApiError';
	}
}

export type RequestOptions = {
	method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
	body?: unknown;
	headers?: Record<string, string>;
	/**
	 * Retained for API compatibility but no longer needed in practice:
	 * authentication rides on the HttpOnly session cookies that the
	 * browser attaches automatically thanks to
	 * `credentials: 'include'`. The flag is kept so call sites that
	 * pass `auth: false` still type-check; it just changes nothing.
	 */
	auth?: boolean;
};

async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
	const { method = 'GET', body, headers = {} } = opts;
	const init: RequestInit = {
		method,
		headers: { ...headers },
		// Cookies aren't sent on same-origin fetches by default in
		// some browsers when the request opts into custom headers; be
		// explicit so the HttpOnly session cookies always travel.
		credentials: 'include'
	};
	if (body !== undefined) {
		(init.headers as Record<string, string>)['content-type'] = 'application/json';
		init.body = JSON.stringify(body);
	}
	const resp = await fetch(path, init);
	if (!resp.ok) {
		let bodyJson: unknown = undefined;
		let message = `${resp.status} ${resp.statusText}`;
		let code = 'http_error';
		try {
			bodyJson = await resp.json();
			if (bodyJson && typeof bodyJson === 'object') {
				const b = bodyJson as { code?: string; message?: string };
				if (b.code) code = b.code;
				if (b.message) message = b.message;
			}
		} catch {
			/* response wasn't JSON; keep the status-line message */
		}
		throw new ApiError(resp.status, code, message, bodyJson);
	}
	// 204 No Content has no body to parse.
	if (resp.status === 204) return undefined as T;
	const text = await resp.text();
	if (!text) return undefined as T;
	return JSON.parse(text) as T;
}

export const api = {
	get: <T>(path: string, opts?: RequestOptions) => request<T>(path, { ...opts, method: 'GET' }),
	post: <T>(path: string, body?: unknown, opts?: RequestOptions) =>
		request<T>(path, { ...opts, method: 'POST', body }),
	put: <T>(path: string, body?: unknown, opts?: RequestOptions) =>
		request<T>(path, { ...opts, method: 'PUT', body }),
	patch: <T>(path: string, body?: unknown, opts?: RequestOptions) =>
		request<T>(path, { ...opts, method: 'PATCH', body }),
	delete: <T>(path: string, opts?: RequestOptions) =>
		request<T>(path, { ...opts, method: 'DELETE' })
};

// ---- typed response shapes ----

export type MasterAdmin = {
	id: string;
	username: string;
	email: string | null;
	name: string | null;
};

export type MasterLoginResponse = {
	access_token: string;
	refresh_token: string;
	admin: MasterAdmin;
};

export type Realm = {
	id: string;
	name: string;
	created_at: string;
};

export type App = {
	id: string;
	name: string;
	created_at: string;
};

export type CollectionKind = 'base' | 'auth' | 'view';

/**
 * Tagged `FieldType` mirroring the Rust enum in `rustbase-core::schema`.
 * The `kind` discriminator is what serde uses on the wire; per-variant
 * options are flattened onto the same object.
 */
export type FieldType =
	| { kind: 'text'; min?: number; max?: number }
	| { kind: 'number'; min?: number; max?: number }
	| { kind: 'bool' }
	| { kind: 'email' }
	| { kind: 'url' }
	| { kind: 'date' }
	| { kind: 'json' }
	| { kind: 'relation'; target: string; cascade_delete?: boolean }
	| { kind: 'file'; max_size?: number; mime_types?: string[] };

export type Field = {
	name: string;
	required?: boolean;
	unique?: boolean;
} & FieldType;

export type Schema = {
	id: string;
	kind: CollectionKind;
	fields: Field[];
};

export type Collection = {
	id: string;
	kind: CollectionKind;
	schema: Schema;
	created_at: string;
	updated_at: string;
};

/**
 * Record shape on the wire — `id`, `collection`, the field map under
 * `fields`, plus the two timestamp columns the records table always
 * carries.
 */
export type RecordRow = {
	id: string;
	collection: string;
	fields: Record<string, unknown>;
	created_at: string;
	updated_at: string;
};

export type RecordListResponse = {
	items: RecordRow[];
	page: number;
	per_page: number;
	total_items: number;
	total_pages: number;
};

// ---- admin user management ----

export type AdminUser = {
	id: string;
	email: string;
	verified: boolean;
	has_password: boolean;
	last_login: string | null;
	created_at: string;
};

export type AdminUserListResponse = {
	items: AdminUser[];
	page: number;
	per_page: number;
	total_items: number;
	total_pages: number;
};

export type AdminTotpStatus = {
	enabled: boolean;
	enrolled_at: string;
	confirmed_at: string | null;
};

export type AdminOAuthLink = {
	provider: string;
	provider_user_id: string;
};

export type AdminUserDetail = AdminUser & {
	totp: AdminTotpStatus | null;
	oauth_links: AdminOAuthLink[];
};

// ---- OAuth provider admin ----

/**
 * Provider config returned by GET. The secret never appears here —
 * it's only ever inbound on PUT.
 */
export type OAuthProviderConfig = {
	auth_url: string;
	token_url: string;
	userinfo_url: string;
	scopes: string[];
	userinfo_id_field: string;
	userinfo_email_field: string;
};

export type OAuthProvider = {
	provider: string;
	client_id: string;
	config: OAuthProviderConfig;
};

/**
 * PUT body shape. `client_secret` is optional on edit — when absent /
 * empty the server reuses the existing ciphertext. Create-without-
 * secret returns 400.
 */
export type OAuthProviderPut = {
	client_id: string;
	client_secret?: string;
	config: OAuthProviderConfig;
};

// ---- hierarchical policies ----

/**
 * Wire-format PolicySpec — tagged union mirroring the Rust enum in
 * `rustbase-core::config`.
 */
export type PolicySpec =
	| { kind: 'range'; min: number; max: number }
	| { kind: 'toggle'; state: 'open'; default: boolean }
	| { kind: 'toggle'; state: 'locked'; value: boolean }
	| { kind: 'enum_set'; allowed: string[] }
	| { kind: 'free' };

export type PolicyResponse = {
	field: string;
	spec: PolicySpec;
	updated_at: string;
};

export type ClampOutcome = {
	realm: string;
	app: string | null;
	field: string;
	before: PolicySpec;
	after: PolicySpec;
};

export type PutPolicyResponse = {
	field: string;
	spec: PolicySpec;
	cascaded: ClampOutcome[];
};

// ---- JS/TS hook source files ----

export type HookFile = {
	filename: string;
	size: number;
	updated_at: string;
};

export type HookFileBody = {
	filename: string;
	source: string;
	size: number;
	updated_at: string;
};

export type ReloadOutcome = {
	loaded: number;
	errors: string[];
};

export type PutHookResponse = {
	file: HookFileBody;
	reload: ReloadOutcome;
};

// ---- per-app file storage ----

export type FileMeta = {
	id: string;
	filename: string;
	mime: string | null;
	size: number;
	created_at: string;
};

// ---- audit log ----

export type AuditEntry = {
	id: number;
	ts: string;
	actor: string | null;
	action: string;
	target: string | null;
	/** Server-decoded JSON; `null` when the stored column was empty or invalid. */
	details: unknown;
};

export type AuditListResponse = {
	items: AuditEntry[];
	page: number;
	per_page: number;
	total_items: number;
	total_pages: number;
};
