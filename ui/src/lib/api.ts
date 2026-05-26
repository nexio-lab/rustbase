/**
 * Minimal fetch wrapper for the RustBase REST API.
 *
 * - Same-origin in production (the SvelteKit bundle is served by the
 *   Rust binary at `/_/`, the API at `/api`, `/_/setup`, `/_/auth`).
 * - Same-origin in dev too — the Vite proxy in `vite.config.ts`
 *   forwards those paths to `localhost:8080`.
 *
 * Every call attaches the access token from the auth store (when set)
 * as `Authorization: Bearer <jwt>`. Non-2xx responses become a thrown
 * `ApiError` so callers can `try { … } catch (e) { … }` with a
 * structured shape and don't have to repeat the same `.ok` / `.json`
 * dance everywhere.
 */

import { auth } from './auth.svelte';

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
	/** When false, skip the `Authorization` header even if a token is set.
	 *  Useful for `/login`, `/setup`, `/auth/users/register`, etc. */
	auth?: boolean;
};

async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
	const { method = 'GET', body, headers = {}, auth: withAuth = true } = opts;
	const init: RequestInit = { method, headers: { ...headers } };
	if (body !== undefined) {
		(init.headers as Record<string, string>)['content-type'] = 'application/json';
		init.body = JSON.stringify(body);
	}
	if (withAuth && auth.token) {
		(init.headers as Record<string, string>)['authorization'] = `Bearer ${auth.token}`;
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
	email: string;
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
