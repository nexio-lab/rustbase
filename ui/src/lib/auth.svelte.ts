/**
 * Session store — JWT + refresh token + the principal identity, kept
 * in localStorage so a tab reload doesn't kick the user back to the
 * login screen.
 *
 * Trade-off: XSS in the dashboard would let an attacker read the
 * tokens. The alternative (httponly cookies) would need the server
 * to issue a cookie alongside the JWT and would couple the dashboard
 * to a specific deployment URL — not worth it for an admin tool
 * served from the same origin as the API. The dashboard sandbox is
 * tight (no user-provided HTML rendering yet), so this is acceptable.
 */

import type { MasterAdmin } from './api';

const STORAGE_KEY = 'rustbase.session.v1';

type SessionShape = {
	access_token: string;
	refresh_token: string;
	role: 'master' | 'realm' | 'app' | 'user';
	admin?: MasterAdmin;
	/** Which realm the principal is bound to, if any. */
	realm?: string;
};

function load(): SessionShape | null {
	if (typeof localStorage === 'undefined') return null;
	const raw = localStorage.getItem(STORAGE_KEY);
	if (!raw) return null;
	try {
		return JSON.parse(raw) as SessionShape;
	} catch {
		localStorage.removeItem(STORAGE_KEY);
		return null;
	}
}

function persist(s: SessionShape | null) {
	if (typeof localStorage === 'undefined') return;
	if (s) localStorage.setItem(STORAGE_KEY, JSON.stringify(s));
	else localStorage.removeItem(STORAGE_KEY);
}

class Auth {
	#session = $state<SessionShape | null>(load());

	get session() {
		return this.#session;
	}
	get token() {
		return this.#session?.access_token ?? null;
	}
	get isAuthenticated() {
		return this.#session !== null;
	}
	get isMaster() {
		return this.#session?.role === 'master';
	}
	get admin() {
		return this.#session?.admin ?? null;
	}

	setMasterSession(login: { access_token: string; refresh_token: string; admin: MasterAdmin }) {
		this.#session = {
			access_token: login.access_token,
			refresh_token: login.refresh_token,
			role: 'master',
			admin: login.admin
		};
		persist(this.#session);
	}

	clear() {
		this.#session = null;
		persist(null);
	}
}

export const auth = new Auth();
