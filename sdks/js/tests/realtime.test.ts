import { describe, expect, it, vi } from 'vitest';
import {
	dispatchRealtimeEvent,
	RustBase,
	Subscription,
	type RealtimeEvent,
	type SubscriptionListeners,
} from '../src/index.js';

/**
 * Realtime wrapper tests. The hard part — actually exchanging WS
 * frames — is locked by Playwright in the main repo's e2e suite.
 * Here we cover the pieces that are pure (URL building, event
 * dispatch) and the reconnect-on-close contract (via an injectable
 * `WebSocketImpl` mock).
 */

describe('dispatchRealtimeEvent', () => {
	it('routes record_created to the right listener', () => {
		const onCreated = vi.fn();
		const onUpdated = vi.fn();
		const onDeleted = vi.fn();
		const listeners: SubscriptionListeners = {
			record_created: onCreated,
			record_updated: onUpdated,
			record_deleted: onDeleted,
		};
		const event: RealtimeEvent = {
			kind: 'record_created',
			record: {
				id: 'r1',
				collection: 'notes',
				fields: { title: 'hi' },
				created_at: '2026-01-01T00:00:00Z',
				updated_at: '2026-01-01T00:00:00Z',
			},
		};
		dispatchRealtimeEvent(event, listeners);
		expect(onCreated).toHaveBeenCalledOnce();
		expect(onUpdated).not.toHaveBeenCalled();
		expect(onDeleted).not.toHaveBeenCalled();
	});

	it('routes record_deleted with the id only', () => {
		const onDeleted = vi.fn();
		dispatchRealtimeEvent({ kind: 'record_deleted', id: 'r1' }, { record_deleted: onDeleted });
		expect(onDeleted).toHaveBeenCalledWith('r1');
	});

	it('silently ignores an unknown kind (forward-compat)', () => {
		const onCreated = vi.fn();
		// Force a future-kind event past the type system to confirm
		// the dispatcher does not throw on it.
		dispatchRealtimeEvent(
			{ kind: 'future_thing' as 'record_created', record: {} as never } as RealtimeEvent,
			{ record_created: onCreated },
		);
		expect(onCreated).not.toHaveBeenCalled();
	});
});

describe('Subscription.buildUrl', () => {
	it('rewrites http → ws and carries token + filter', () => {
		const rb = new RustBase({
			baseUrl: 'http://h:8080',
			workspace: 'acme',
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u', email: 'a@b.c', verified: true },
			},
			// Inject a no-op WebSocketImpl so the subscription
			// doesn't try to open a real socket during construction.
			fetch: vi.fn() as unknown as typeof fetch,
		});
		const sub = new Subscription(rb, 'mobile', 'notes', {
			filter: 'pinned = true',
			WebSocketImpl: NoopWebSocket as unknown as typeof WebSocket,
			setTimeout: () => 0,
		});
		const url = new URL(sub.buildUrl('the-token'));
		expect(url.protocol).toBe('ws:');
		expect(url.host).toBe('h:8080');
		expect(url.pathname).toBe(
			'/api/workspaces/acme/apps/mobile/collections/notes/events/ws',
		);
		expect(url.searchParams.get('token')).toBe('the-token');
		expect(url.searchParams.get('filter')).toBe('pinned = true');
		sub.close();
	});

	it('rewrites https → wss', () => {
		const rb = new RustBase({
			baseUrl: 'https://api.example.com',
			workspace: 'acme',
			session: {
				accessToken: 'at',
				refreshToken: 'rt',
				user: { id: 'u', email: 'a@b.c', verified: true },
			},
			fetch: vi.fn() as unknown as typeof fetch,
		});
		const sub = new Subscription(rb, 'mobile', 'notes', {
			WebSocketImpl: NoopWebSocket as unknown as typeof WebSocket,
			setTimeout: () => 0,
		});
		expect(sub.buildUrl('t').startsWith('wss://api.example.com/')).toBe(true);
		sub.close();
	});
});

describe('Subscription lifecycle', () => {
	it('fires open then dispatches a single message', () => {
		const rb = newClientWithSession();
		const ws = new FakeWebSocket();
		const onOpen = vi.fn();
		const onCreated = vi.fn();
		const sub = new Subscription(rb, 'mobile', 'notes', {
			WebSocketImpl: makeImpl(ws),
			setTimeout: () => 0,
		})
			.on('open', onOpen)
			.on('record_created', onCreated);
		ws.fireOpen();
		expect(onOpen).toHaveBeenCalled();
		ws.fireMessage(
			JSON.stringify({
				kind: 'record_created',
				record: {
					id: 'r1',
					collection: 'notes',
					fields: { title: 'x' },
					created_at: '2026-01-01T00:00:00Z',
					updated_at: '2026-01-01T00:00:00Z',
				},
			}),
		);
		expect(onCreated).toHaveBeenCalledOnce();
		sub.close();
	});

	it('schedules a reconnect when the socket closes without explicit close', () => {
		const rb = newClientWithSession();
		const ws = new FakeWebSocket();
		let scheduledDelay = -1;
		const onClose = vi.fn();
		new Subscription(rb, 'mobile', 'notes', {
			WebSocketImpl: makeImpl(ws),
			setTimeout: (_cb, ms) => {
				scheduledDelay = ms;
				return 0;
			},
		}).on('close', onClose);
		ws.fireClose(1006, 'abnormal');
		expect(onClose).toHaveBeenCalledOnce();
		const call = onClose.mock.calls[0]![0] as { code: number; willReconnect: boolean };
		expect(call.code).toBe(1006);
		expect(call.willReconnect).toBe(true);
		expect(scheduledDelay).toBeGreaterThan(0);
	});

	it('does NOT schedule a reconnect after `close()`', () => {
		const rb = newClientWithSession();
		const ws = new FakeWebSocket();
		const onClose = vi.fn();
		let scheduledDelay = -1;
		const sub = new Subscription(rb, 'mobile', 'notes', {
			WebSocketImpl: makeImpl(ws),
			setTimeout: (_cb, ms) => {
				scheduledDelay = ms;
				return 0;
			},
		}).on('close', onClose);
		sub.close();
		ws.fireClose(1000, 'normal');
		// The listener still fires because we wired it before close
		// took effect on the underlying socket — but it MUST tell us
		// no reconnect is coming.
		const call = onClose.mock.calls[0]![0] as { willReconnect: boolean };
		expect(call.willReconnect).toBe(false);
		expect(scheduledDelay).toBe(-1);
	});

	it('[garde-existant] opens no socket without an active session', () => {
		const rb = new RustBase({
			baseUrl: 'http://h',
			workspace: 'acme',
			fetch: vi.fn() as unknown as typeof fetch,
		});
		const ws = new FakeWebSocket();
		const onError = vi.fn();
		new Subscription(rb, 'mobile', 'notes', {
			WebSocketImpl: makeImpl(ws),
			setTimeout: () => 0,
		}).on('error', onError);
		// The Subscription constructor invokes `connect()`
		// synchronously; the error listener is registered AFTER, so
		// the inner error never reaches it. The contract under test
		// is that no WS was opened — the FakeWebSocket counter is
		// the witness.
		// We'd want to register the listener before connect() in a
		// real app; see the README.
		expect(ws.openedCount).toBe(0);
		expect(onError).not.toHaveBeenCalled();
	});
});

// ---------- helpers ----------

function newClientWithSession(): RustBase {
	return new RustBase({
		baseUrl: 'http://h',
		workspace: 'acme',
		session: {
			accessToken: 'at',
			refreshToken: 'rt',
			user: { id: 'u', email: 'a@b.c', verified: true },
		},
		fetch: vi.fn() as unknown as typeof fetch,
	});
}

function makeImpl(ws: FakeWebSocket): typeof WebSocket {
	// Subscription invokes `new this.WebSocketImpl(url)` once.
	return function FakeImpl(_url: string) {
		ws.openedCount += 1;
		return ws;
	} as unknown as typeof WebSocket;
}

/** Minimal WebSocket stand-in: enough for the lifecycle paths. */
class FakeWebSocket {
	openedCount = 0;
	private listeners: { [k: string]: ((ev: Event) => void)[] } = {};
	addEventListener(name: string, cb: (ev: Event) => void): void {
		(this.listeners[name] ??= []).push(cb);
	}
	close(): void {
		// no-op
	}
	fireOpen(): void {
		this.fire('open', new Event('open'));
	}
	fireMessage(data: string): void {
		const ev = new MessageEvent('message', { data });
		this.fire('message', ev);
	}
	fireClose(code: number, reason: string): void {
		const ev = new CloseEvent('close', { code, reason });
		this.fire('close', ev);
	}
	private fire(name: string, ev: Event): void {
		const handlers = this.listeners[name] ?? [];
		for (const h of handlers) h(ev);
	}
}

class NoopWebSocket {
	addEventListener(): void {
		// no-op
	}
	close(): void {
		// no-op
	}
}
