/**
 * Session store — keeps only the **non-secret** identity blob
 * (role / admin profile / realm scope) so the SPA can route + render
 * conditionally. The actual JWT and refresh token live in HttpOnly
 * cookies (`rb_at`, `rb_rt`) issued by the server on login and
 * cleared on logout. JS cannot read them, which kills the XSS
 * token-theft surface the old `localStorage` design had.
 */

import type { MasterAdmin } from './api';

const STORAGE_KEY = 'rustbase.session.v2';

type SessionShape = {
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
	get isAuthenticated() {
		return this.#session !== null;
	}
	get isMaster() {
		return this.#session?.role === 'master';
	}
	get admin() {
		return this.#session?.admin ?? null;
	}

	setMasterSession(login: { admin: MasterAdmin }) {
		this.#session = {
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
