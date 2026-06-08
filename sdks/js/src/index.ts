/**
 * @rustbase/client — official JavaScript / TypeScript SDK.
 *
 * Idiomatic wrapper around the RustBase REST API. Shapes mirror the
 * canonical spec at `docs/reference/openapi.yaml`. The client owns
 * the session — store the access + refresh token on `auth.login`,
 * transparently rotate when the access token nears expiry, and
 * surface a typed error if a request is refused.
 *
 *     import { RustBase } from '@rustbase/client';
 *
 *     const rb = new RustBase({
 *         baseUrl:   'https://api.example.com',
 *         workspace: 'acme',
 *     });
 *     await rb.auth.login({ email, password });
 *     const list = await rb.app('mobile').collection('notes').list({
 *         filter:  'pinned = true',
 *         sort:    '-updated_at',
 *         perPage: 30,
 *     });
 */

// ---------- Public types ----------

export type RustBaseOptions = {
	baseUrl: string;
	workspace: string;
	/**
	 * Optional fetch override — pass `globalThis.fetch.bind(globalThis)` to
	 * preserve `this` in environments where it matters, or your own
	 * instrumented wrapper. Defaults to `globalThis.fetch`.
	 */
	fetch?: typeof fetch;
	/**
	 * Optional session bootstrap — restore a previously persisted
	 * session at construction time. The SDK does not touch
	 * `localStorage`; persistence is the caller's responsibility.
	 */
	session?: Session | null;
	/**
	 * Called every time the session changes (login, refresh, logout).
	 * Use this to persist the new tokens. The argument is `null` on
	 * logout.
	 */
	onSessionChange?: (session: Session | null) => void;
};

export type Session = {
	accessToken: string;
	refreshToken: string;
	user: UserPublic;
};

export type UserPublic = {
	id: string;
	email: string;
	verified: boolean;
};

export type RegisterRequest = {
	email: string;
	password: string;
};

export type LoginRequest = {
	email: string;
	password: string;
};

/**
 * A `userLogin` response carries either tokens (and we promote it to
 * a `Session`) or an MFA challenge.
 */
export type LoginResult =
	| { kind: 'session'; session: Session }
	| { kind: 'mfa'; mfaToken: string };

export type RecordRow = {
	id: string;
	collection: string;
	fields: { [key: string]: unknown };
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

export type ListQuery = {
	filter?: string;
	sort?: string;
	page?: number;
	perPage?: number;
};

export type FileMeta = {
	id: string;
	mime: string;
	size: number;
	url: string;
	sha256?: string;
};

/**
 * Wire shape of the events delivered over the realtime WebSocket.
 * Mirrors the server's `#[serde(tag = "kind", rename_all =
 * "snake_case")] RealtimeEvent` enum.
 */
export type RealtimeEvent =
	| { kind: 'record_created'; record: RecordRow }
	| { kind: 'record_updated'; record: RecordRow }
	| { kind: 'record_deleted'; id: string };

export type SubscriptionListeners = {
	/** Fired once on every successful connect (including reconnects). */
	open?: () => void;
	/**
	 * Fired on every disconnect. The wrapper will reconnect automatically
	 * unless `close()` was called on the subscription.
	 */
	close?: (info: { code: number; reason: string; willReconnect: boolean }) => void;
	/** Generic error channel — parse failures, missing session, etc. */
	error?: (err: Error) => void;
	record_created?: (record: RecordRow) => void;
	record_updated?: (record: RecordRow) => void;
	record_deleted?: (id: string) => void;
};

export type SubscribeOptions = {
	filter?: string;
	/**
	 * Optional override for the WebSocket constructor — defaults to
	 * `globalThis.WebSocket`. Set this in tests or in non-browser
	 * environments without a global WebSocket.
	 */
	WebSocketImpl?: typeof WebSocket;
	/** Optional override for `setTimeout`, for tests. */
	setTimeout?: (cb: () => void, ms: number) => unknown;
};

// ---------- Errors ----------

/**
 * Thrown by every SDK call when the server returns a non-2xx response
 * or a request fails before reaching the server. `code` mirrors the
 * server-side `ErrorBody.code` (see `docs/reference/errors`) when
 * available, or one of the SDK-only sentinels for transport-layer
 * failures.
 */
export class RustBaseError extends Error {
	readonly status: number;
	readonly code: string;
	readonly body?: unknown;

	constructor(status: number, code: string, message: string, body?: unknown) {
		super(message);
		this.name = 'RustBaseError';
		this.status = status;
		this.code = code;
		this.body = body;
	}
}

// ---------- Implementation ----------

export class RustBase {
	private readonly baseUrl: string;
	readonly workspace: string;
	private readonly fetchImpl: typeof fetch;
	private readonly onSessionChange: ((s: Session | null) => void) | undefined;
	private session: Session | null;

	readonly auth: AuthApi;

	constructor(options: RustBaseOptions) {
		this.baseUrl = stripTrailingSlash(options.baseUrl);
		this.workspace = options.workspace;
		this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
		this.onSessionChange = options.onSessionChange;
		this.session = options.session ?? null;
		this.auth = new AuthApi(this);
	}

	/** Scope subsequent calls to a specific app. */
	app(id: string): AppHandle {
		return new AppHandle(this, id);
	}

	/** Current session, or `null` if logged out. */
	get currentSession(): Session | null {
		return this.session;
	}

	/** Replace the current session and emit `onSessionChange`. */
	setSession(next: Session | null): void {
		this.session = next;
		this.onSessionChange?.(next);
	}

	/**
	 * Internal: low-level request. Adds the workspace base path,
	 * carries the access token when authenticated, attempts ONE
	 * refresh on 401 if a refresh token is available, then surfaces
	 * the result or throws `RustBaseError`.
	 */
	async request<T>(
		method: string,
		path: string,
		init: { body?: unknown; query?: Record<string, string | number | undefined>; multipart?: FormData } = {},
	): Promise<T> {
		const url = this.buildUrl(path, init.query);
		const res = await this.fetchOnce(method, url, init, /* allowRetry */ true);
		const contentType = res.headers.get('content-type') ?? '';
		if (res.status === 204) {
			return undefined as T;
		}
		if (contentType.includes('application/json')) {
			return (await res.json()) as T;
		}
		// Binary or other — return the response itself for callers that need it.
		return res as unknown as T;
	}

	private async fetchOnce(
		method: string,
		url: string,
		init: { body?: unknown; multipart?: FormData },
		allowRetry: boolean,
	): Promise<Response> {
		const headers: HeadersInit = {};
		let body: BodyInit | undefined;
		if (init.multipart) {
			body = init.multipart;
		} else if (init.body !== undefined) {
			headers['Content-Type'] = 'application/json';
			body = JSON.stringify(init.body);
		}
		if (this.session?.accessToken) {
			headers['Authorization'] = `Bearer ${this.session.accessToken}`;
		}

		let res: Response;
		try {
			res = await this.fetchImpl(url, { method, headers, body });
		} catch (e) {
			throw new RustBaseError(0, 'network', e instanceof Error ? e.message : String(e));
		}

		if (res.status === 401 && allowRetry && this.session?.refreshToken) {
			// Try a single refresh, then replay.
			const refreshed = await this.refreshOnce();
			if (refreshed) {
				return this.fetchOnce(method, url, init, false);
			}
		}

		if (!res.ok) {
			let code = 'unknown';
			let message = `HTTP ${res.status}`;
			let parsed: unknown;
			try {
				parsed = await res.clone().json();
				if (parsed && typeof parsed === 'object') {
					const obj = parsed as { code?: unknown; message?: unknown };
					if (typeof obj.code === 'string') code = obj.code;
					if (typeof obj.message === 'string') message = obj.message;
				}
			} catch {
				// non-JSON body — keep the defaults.
			}
			throw new RustBaseError(res.status, code, message, parsed);
		}

		return res;
	}

	/**
	 * Public-but-not-typed-as-public: lets `auth.refresh()` call the
	 * internal one-shot refresh. Returns true on success, false on
	 * failure (caller sees the session change via
	 * `onSessionChange(null)`).
	 */
	async refreshFromOutside(): Promise<boolean> {
		return this.refreshOnce();
	}

	/**
	 * Internal: one-shot refresh attempt. Returns true on success
	 * (session updated) or false on failure. Failure does NOT throw —
	 * the caller surfaces the original 401 to keep the trace
	 * meaningful.
	 */
	private async refreshOnce(): Promise<boolean> {
		const rt = this.session?.refreshToken;
		if (!rt) return false;
		const url = this.buildUrl(`/api/workspaces/${this.workspace}/auth/refresh`);
		try {
			const res = await this.fetchImpl(url, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ refresh_token: rt }),
			});
			if (!res.ok) {
				this.setSession(null);
				return false;
			}
			const body = (await res.json()) as { access_token: string; refresh_token: string };
			const current = this.session;
			if (!current) return false;
			this.setSession({
				accessToken: body.access_token,
				refreshToken: body.refresh_token,
				user: current.user,
			});
			return true;
		} catch {
			return false;
		}
	}

	private buildUrl(path: string, query?: Record<string, string | number | undefined>): string {
		let url = `${this.baseUrl}${path}`;
		if (query) {
			const params = new URLSearchParams();
			for (const [k, v] of Object.entries(query)) {
				if (v === undefined) continue;
				params.set(k, String(v));
			}
			const qs = params.toString();
			if (qs) url += `?${qs}`;
		}
		return url;
	}

	/**
	 * Build a WebSocket URL by rewriting the scheme (`http` → `ws`,
	 * `https` → `wss`) on top of the configured base. Used by the
	 * realtime subscription.
	 */
	wsUrl(path: string, query?: Record<string, string | undefined>): string {
		const wsBase = this.baseUrl.replace(/^http/, 'ws');
		let url = `${wsBase}${path}`;
		if (query) {
			const params = new URLSearchParams();
			for (const [k, v] of Object.entries(query)) {
				if (v === undefined) continue;
				params.set(k, v);
			}
			const qs = params.toString();
			if (qs) url += `?${qs}`;
		}
		return url;
	}
}

class AuthApi {
	constructor(private readonly client: RustBase) {}

	/** `POST /auth/users/register`. */
	async register(req: RegisterRequest): Promise<UserPublic> {
		return this.client.request<UserPublic>(
			'POST',
			`/api/workspaces/${this.client.workspace}/auth/users/register`,
			{ body: req },
		);
	}

	/** `POST /auth/verification/request`. */
	async requestVerification(req: { email: string }): Promise<void> {
		await this.client.request<void>(
			'POST',
			`/api/workspaces/${this.client.workspace}/auth/verification/request`,
			{ body: req },
		);
	}

	/** `POST /auth/verification/confirm`. */
	async confirmVerification(req: { email: string; code: string }): Promise<void> {
		await this.client.request<void>(
			'POST',
			`/api/workspaces/${this.client.workspace}/auth/verification/confirm`,
			{ body: req },
		);
	}

	/**
	 * `POST /auth/users/login`. Promotes the response to a
	 * `LoginResult` — either a `session` discriminant (tokens
	 * already stored on the client) or `mfa` (caller must follow up
	 * with `completeMfa`).
	 */
	async login(req: LoginRequest): Promise<LoginResult> {
		const body = await this.client.request<
			| { access_token: string; refresh_token: string; user: UserPublic }
			| { mfa_required: true; mfa_token: string }
		>('POST', `/api/workspaces/${this.client.workspace}/auth/users/login`, { body: req });

		if ('mfa_required' in body) {
			return { kind: 'mfa', mfaToken: body.mfa_token };
		}
		const session: Session = {
			accessToken: body.access_token,
			refreshToken: body.refresh_token,
			user: body.user,
		};
		this.client.setSession(session);
		return { kind: 'session', session };
	}

	/** `POST /auth/users/login/totp` — complete an MFA-gated login. */
	async completeMfa(mfaToken: string, code: string): Promise<Session> {
		const body = await this.client.request<{
			access_token: string;
			refresh_token: string;
			user: UserPublic;
		}>('POST', `/api/workspaces/${this.client.workspace}/auth/users/login/totp`, {
			body: { mfa_token: mfaToken, code },
		});
		const session: Session = {
			accessToken: body.access_token,
			refreshToken: body.refresh_token,
			user: body.user,
		};
		this.client.setSession(session);
		return session;
	}

	/** `POST /auth/logout` — revoke the session server-side and drop tokens locally. */
	async logout(): Promise<void> {
		try {
			await this.client.request<void>(
				'POST',
				`/api/workspaces/${this.client.workspace}/auth/logout`,
			);
		} finally {
			this.client.setSession(null);
		}
	}

	/**
	 * Rotate the refresh token explicitly. The regular request loop
	 * already refreshes on 401, but the realtime wrapper needs to be
	 * able to refresh BEFORE reconnecting after an auth-related close
	 * — by then no HTTP request has surfaced the 401 yet. Returns
	 * `true` on success (a new session is now in place), `false`
	 * otherwise (the session is cleared and `onSessionChange(null)`
	 * has fired).
	 */
	async refresh(): Promise<boolean> {
		return this.client.refreshFromOutside();
	}
}

export class AppHandle {
	constructor(
		readonly client: RustBase,
		readonly id: string,
	) {}

	/** Scope to a collection inside this app. */
	collection(slug: string): CollectionHandle {
		return new CollectionHandle(this.client, this.id, slug);
	}

	/** File operations on this app. */
	get files(): FilesApi {
		return new FilesApi(this.client, this.id);
	}
}

export class CollectionHandle {
	constructor(
		private readonly client: RustBase,
		private readonly app: string,
		readonly slug: string,
	) {}

	private get base(): string {
		return `/api/workspaces/${this.client.workspace}/apps/${this.app}/collections/${this.slug}`;
	}

	async list(query: ListQuery = {}): Promise<RecordListResponse> {
		return this.client.request<RecordListResponse>('GET', `${this.base}/records`, {
			query: {
				page: query.page,
				per_page: query.perPage,
				filter: query.filter,
				sort: query.sort,
			},
		});
	}

	async get(id: string): Promise<RecordRow> {
		return this.client.request<RecordRow>('GET', `${this.base}/records/${encodeURIComponent(id)}`);
	}

	async create(fields: { [key: string]: unknown }): Promise<RecordRow> {
		return this.client.request<RecordRow>('POST', `${this.base}/records`, { body: fields });
	}

	async update(id: string, fields: { [key: string]: unknown }): Promise<RecordRow> {
		return this.client.request<RecordRow>(
			'PATCH',
			`${this.base}/records/${encodeURIComponent(id)}`,
			{ body: fields },
		);
	}

	async delete(id: string): Promise<void> {
		await this.client.request<void>(
			'DELETE',
			`${this.base}/records/${encodeURIComponent(id)}`,
		);
	}

	/**
	 * Open a realtime subscription against this collection's WS
	 * endpoint. Returns a `Subscription` you call `.on(event, fn)`
	 * on, and `.close()` to tear down.
	 *
	 *     const sub = notes.subscribe({ filter: 'pinned = true' });
	 *     sub.on('record_created', (r) => …);
	 *     sub.on('record_updated', (r) => …);
	 *     sub.on('record_deleted', (id) => …);
	 *     // later:
	 *     sub.close();
	 *
	 * The wrapper reconnects automatically with jittered exponential
	 * backoff on close, and proactively refreshes the access token
	 * before reconnect if the server closes with a policy-violation
	 * code (1008 / 4001 / 4003).
	 */
	subscribe(opts: SubscribeOptions = {}): Subscription {
		return new Subscription(this.client, this.app, this.slug, opts);
	}
}

/**
 * Realtime subscription handle. Owns one WebSocket at a time, plus
 * the reconnect loop. Built by `CollectionHandle.subscribe`.
 */
export class Subscription {
	private ws: WebSocket | null = null;
	private listeners: SubscriptionListeners = {};
	private explicitClose = false;
	private retryCount = 0;
	private readonly WebSocketImpl: typeof WebSocket;
	private readonly setTimeoutImpl: (cb: () => void, ms: number) => unknown;

	constructor(
		private readonly client: RustBase,
		private readonly app: string,
		private readonly slug: string,
		opts: SubscribeOptions = {},
	) {
		this.filter = opts.filter;
		this.WebSocketImpl =
			opts.WebSocketImpl ?? (globalThis.WebSocket as typeof WebSocket | undefined)!;
		this.setTimeoutImpl = opts.setTimeout ?? ((cb, ms) => setTimeout(cb, ms));
		if (!this.WebSocketImpl) {
			throw new RustBaseError(
				0,
				'no_websocket',
				'no WebSocket constructor available — pass `WebSocketImpl` in subscribe options',
			);
		}
		this.connect();
	}

	private readonly filter: string | undefined;

	/** Register a listener for one event type. Replaces any prior handler. */
	on<K extends keyof SubscriptionListeners>(
		event: K,
		handler: SubscriptionListeners[K],
	): this {
		this.listeners[event] = handler;
		return this;
	}

	/**
	 * Close the subscription and stop the reconnect loop. Idempotent.
	 */
	close(): void {
		this.explicitClose = true;
		if (this.ws) {
			const w = this.ws;
			this.ws = null;
			try {
				w.close();
			} catch {
				// already closing / closed — ignore.
			}
		}
	}

	/**
	 * Build the WS URL for the current session. Exposed for tests so
	 * the URL contract can be locked without booting a real socket.
	 */
	buildUrl(token: string): string {
		const path = `/api/workspaces/${this.client.workspace}/apps/${this.app}/collections/${this.slug}/events/ws`;
		return this.client.wsUrl(path, {
			token,
			filter: this.filter,
		});
	}

	private connect(): void {
		if (this.explicitClose) return;
		const session = this.client.currentSession;
		if (!session) {
			this.listeners.error?.(
				new RustBaseError(401, 'no_session', 'no active session to open a subscription'),
			);
			return;
		}

		const url = this.buildUrl(session.accessToken);
		let ws: WebSocket;
		try {
			ws = new this.WebSocketImpl(url);
		} catch (e) {
			this.listeners.error?.(e instanceof Error ? e : new Error(String(e)));
			this.scheduleReconnect(1006);
			return;
		}
		this.ws = ws;

		ws.addEventListener('open', () => {
			this.retryCount = 0;
			this.listeners.open?.();
		});

		ws.addEventListener('message', (ev: MessageEvent) => {
			try {
				const parsed = JSON.parse(String(ev.data)) as RealtimeEvent;
				dispatchRealtimeEvent(parsed, this.listeners);
			} catch (e) {
				this.listeners.error?.(e instanceof Error ? e : new Error(String(e)));
			}
		});

		ws.addEventListener('close', (ev: CloseEvent) => {
			this.ws = null;
			const code = ev.code;
			const reason = ev.reason;
			this.listeners.close?.({
				code,
				reason,
				willReconnect: !this.explicitClose,
			});
			if (!this.explicitClose) this.scheduleReconnect(code);
		});

		// WS `error` events carry no useful info in browsers; let
		// `close` drive the reconnect and only surface here for
		// debug visibility.
		ws.addEventListener('error', () => {
			this.listeners.error?.(new RustBaseError(0, 'ws_error', 'WebSocket error'));
		});
	}

	private scheduleReconnect(closeCode: number): void {
		if (this.explicitClose) return;
		this.retryCount += 1;

		const refreshFirst = closeCode === 1008 || closeCode === 4001 || closeCode === 4003;
		// Exponential backoff: 500ms, 1s, 2s, ..., capped at 30s, +
		// up to 500ms jitter so a fleet of clients doesn't all
		// reconnect on the same tick after a server restart.
		const base = Math.min(30_000, 500 * Math.pow(2, this.retryCount - 1));
		const delay = base + Math.floor(Math.random() * 500);

		this.setTimeoutImpl(() => {
			if (this.explicitClose) return;
			if (refreshFirst) {
				this.client.auth.refresh().finally(() => this.connect());
			} else {
				this.connect();
			}
		}, delay);
	}
}

/**
 * Pure event dispatcher — exposed for tests. Routes a parsed
 * `RealtimeEvent` to the matching listener; ignores unknown kinds
 * so a future server-side addition doesn't crash older SDKs.
 */
export function dispatchRealtimeEvent(
	event: RealtimeEvent,
	listeners: SubscriptionListeners,
): void {
	switch (event.kind) {
		case 'record_created':
			listeners.record_created?.(event.record);
			return;
		case 'record_updated':
			listeners.record_updated?.(event.record);
			return;
		case 'record_deleted':
			listeners.record_deleted?.(event.id);
			return;
	}
}

export class FilesApi {
	constructor(
		private readonly client: RustBase,
		private readonly app: string,
	) {}

	private get base(): string {
		return `/api/workspaces/${this.client.workspace}/apps/${this.app}/files`;
	}

	/**
	 * Multipart upload. Returns the `FileMeta` to attach to a record
	 * via its file field.
	 */
	async upload(file: Blob | File, fieldName = 'file'): Promise<FileMeta> {
		const form = new FormData();
		form.append(fieldName, file);
		return this.client.request<FileMeta>('POST', this.base, { multipart: form });
	}

	/** Public-facing URL for the binary. */
	serveUrl(id: string): string {
		return `${this.base}/${encodeURIComponent(id)}/serve`;
	}
}

// ---------- Private helpers ----------

function stripTrailingSlash(s: string): string {
	return s.endsWith('/') ? s.slice(0, -1) : s;
}
