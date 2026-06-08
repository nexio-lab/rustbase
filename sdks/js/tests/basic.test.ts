import { describe, expect, it, vi } from 'vitest';
import { RustBase, RustBaseError } from '../src/index.js';

/**
 * Pure unit tests against a hand-rolled `fetch` mock. The point is
 * to lock the wire contract — paths, methods, headers, body shape,
 * 401-triggered refresh — without booting a real RustBase server.
 *
 * Reasoning behind the mock approach: a live integration suite
 * lives in the main repo's `ui/tests/e2e/` and exercises the
 * dashboard end-to-end. The SDK's job is to translate ergonomic
 * calls into the right HTTP requests; a mock fetch is the right
 * altitude for that.
 */

type CapturedCall = { url: string; method: string; headers: Headers; body: BodyInit | undefined };

function mockFetch(responses: Array<{ status: number; body: unknown }>) {
	const calls: CapturedCall[] = [];
	const fn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
		const url = typeof input === 'string' ? input : input.toString();
		calls.push({
			url,
			method: init?.method ?? 'GET',
			headers: new Headers((init?.headers as HeadersInit) ?? {}),
			body: init?.body ?? undefined,
		});
		const next = responses.shift();
		if (!next) throw new Error(`unexpected extra fetch call to ${url}`);
		// A 204 response forbids a body per the Fetch spec — `new Response`
		// throws if we try.
		if (next.status === 204) {
			return new Response(null, { status: 204 });
		}
		return new Response(JSON.stringify(next.body), {
			status: next.status,
			headers: { 'Content-Type': 'application/json' },
		});
	}) as unknown as typeof fetch;
	return { fn, calls };
}

describe('RustBase', () => {
	it('builds the right URL for register', async () => {
		const { fn, calls } = mockFetch([
			{ status: 201, body: { id: 'u1', email: 'a@b.c', verified: false } },
		]);
		const rb = new RustBase({ baseUrl: 'http://h', workspace: 'acme', fetch: fn });
		const user = await rb.auth.register({ email: 'a@b.c', password: 'hunter22' });
		expect(user.id).toBe('u1');
		expect(calls).toHaveLength(1);
		expect(calls[0]!.url).toBe('http://h/api/workspaces/acme/auth/users/register');
		expect(calls[0]!.method).toBe('POST');
		expect(JSON.parse(calls[0]!.body as string)).toEqual({
			email: 'a@b.c',
			password: 'hunter22',
		});
	});

	it('login: tokens path promotes to a session', async () => {
		const { fn } = mockFetch([
			{
				status: 200,
				body: {
					access_token: 'at',
					refresh_token: 'rt',
					user: { id: 'u1', email: 'a@b.c', verified: true },
				},
			},
		]);
		let captured: ReturnType<typeof rb.currentSession> | null = null;
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: fn,
			onSessionChange: (s) => {
				captured = s;
			},
		});
		const result = await rb.auth.login({ email: 'a@b.c', password: 'hunter22' });
		expect(result.kind).toBe('session');
		expect(rb.currentSession?.accessToken).toBe('at');
		expect(captured?.refreshToken).toBe('rt');
	});

	it('login: MFA challenge path does NOT store a session', async () => {
		const { fn } = mockFetch([
			{ status: 200, body: { mfa_required: true, mfa_token: 'mt' } },
		]);
		const rb = new RustBase({ baseUrl: 'http://h', workspace: 'acme', fetch: fn });
		const result = await rb.auth.login({ email: 'a@b.c', password: 'hunter22' });
		expect(result.kind).toBe('mfa');
		expect(rb.currentSession).toBeNull();
	});

	it('list records includes filter, sort, page, per_page in the query', async () => {
		const { fn, calls } = mockFetch([
			{ status: 200, body: { items: [], page: 1, per_page: 30, total_items: 0, total_pages: 0 } },
		]);
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: fn,
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u1', email: 'a@b.c', verified: true },
			},
		});
		await rb.app('mobile').collection('notes').list({
			filter: 'pinned = true',
			sort: '-updated_at',
			page: 2,
			perPage: 50,
		});
		const u = new URL(calls[0]!.url);
		expect(u.pathname).toBe(
			'/api/workspaces/acme/apps/mobile/collections/notes/records',
		);
		expect(u.searchParams.get('filter')).toBe('pinned = true');
		expect(u.searchParams.get('sort')).toBe('-updated_at');
		expect(u.searchParams.get('page')).toBe('2');
		expect(u.searchParams.get('per_page')).toBe('50');
		expect(calls[0]!.headers.get('Authorization')).toBe('Bearer at');
	});

	it('401 triggers a single refresh then replays the original call', async () => {
		const { fn, calls } = mockFetch([
			// First attempt: 401 — server says the access token is dead.
			{ status: 401, body: { code: 'unauthorized', message: 'expired' } },
			// Refresh call.
			{ status: 200, body: { access_token: 'at2', refresh_token: 'rt2' } },
			// Replay of the original list call.
			{ status: 200, body: { items: [], page: 1, per_page: 30, total_items: 0, total_pages: 0 } },
		]);
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: fn,
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u1', email: 'a@b.c', verified: true },
			},
		});
		const list = await rb.app('mobile').collection('notes').list();
		expect(list.items).toEqual([]);
		expect(calls).toHaveLength(3);
		expect(calls[1]!.url).toBe('http://h/api/workspaces/acme/auth/refresh');
		expect(rb.currentSession?.accessToken).toBe('at2');
		expect(rb.currentSession?.refreshToken).toBe('rt2');
	});

	it('throws RustBaseError with code + message from the body on a 4xx', async () => {
		const { fn } = mockFetch([
			{ status: 409, body: { code: 'conflict', message: 'email exists' } },
		]);
		const rb = new RustBase({ baseUrl: 'http://h', workspace: 'acme', fetch: fn });
		await expect(rb.auth.register({ email: 'a@b.c', password: 'hunter22' }))
			.rejects.toMatchObject({
				name: 'RustBaseError',
				status: 409,
				code: 'conflict',
				message: 'email exists',
			} satisfies Partial<RustBaseError>);
	});

	it('logout calls the server then clears the session unconditionally', async () => {
		const { fn } = mockFetch([{ status: 204, body: '' }]);
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: fn,
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u1', email: 'a@b.c', verified: true },
			},
		});
		await rb.auth.logout();
		expect(rb.currentSession).toBeNull();
	});

	it('logout clears the session even if the server call fails', async () => {
		const { fn } = mockFetch([
			{ status: 500, body: { code: 'internal', message: 'oops' } },
		]);
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: fn,
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u1', email: 'a@b.c', verified: true },
			},
		});
		await expect(rb.auth.logout()).rejects.toBeInstanceOf(RustBaseError);
		expect(rb.currentSession).toBeNull();
	});

	it('file upload posts FormData and returns the FileMeta', async () => {
		const { fn, calls } = mockFetch([
			{
				status: 201,
				body: {
					id: 'f1',
					mime: 'image/png',
					size: 1234,
					url: '/api/workspaces/acme/apps/mobile/files/f1/serve',
				},
			},
		]);
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: fn,
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u1', email: 'a@b.c', verified: true },
			},
		});
		const blob = new Blob(['hello'], { type: 'image/png' });
		const meta = await rb.app('mobile').files.upload(blob);
		expect(meta.id).toBe('f1');
		expect(calls[0]!.body).toBeInstanceOf(FormData);
		// JSON Content-Type MUST NOT be set on multipart; the runtime picks
		// the right one (with boundary) from the FormData itself.
		expect(calls[0]!.headers.get('Content-Type')).toBeNull();
	});
});
