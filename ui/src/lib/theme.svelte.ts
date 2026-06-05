// Theme runes — Light / Dark / Auto. Auto follows the OS-level
// `prefers-color-scheme` and re-evaluates when the OS preference
// changes. Explicit "light" or "dark" persists in localStorage and
// overrides the OS until cleared.
//
// Mounted by the root +layout.svelte which threads the resolved
// theme onto the <html> element (`class="dark"` or no class) so
// Tailwind's `dark:` variant lights up app-wide.

const STORAGE_KEY = 'rb_theme';

type ThemeChoice = 'auto' | 'light' | 'dark';

function readStored(): ThemeChoice {
	if (typeof localStorage === 'undefined') return 'auto';
	const raw = localStorage.getItem(STORAGE_KEY);
	if (raw === 'light' || raw === 'dark' || raw === 'auto') return raw;
	return 'auto';
}

function prefersDark(): boolean {
	if (typeof window === 'undefined' || !window.matchMedia) return false;
	return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

class ThemeStore {
	choice = $state<ThemeChoice>(readStored());
	osDark = $state<boolean>(prefersDark());

	resolved = $derived<'light' | 'dark'>(
		this.choice === 'auto' ? (this.osDark ? 'dark' : 'light') : this.choice
	);

	set(c: ThemeChoice) {
		this.choice = c;
		if (typeof localStorage !== 'undefined') {
			if (c === 'auto') localStorage.removeItem(STORAGE_KEY);
			else localStorage.setItem(STORAGE_KEY, c);
		}
	}

	cycle() {
		// auto → light → dark → auto …
		const next: ThemeChoice =
			this.choice === 'auto' ? 'light' : this.choice === 'light' ? 'dark' : 'auto';
		this.set(next);
	}

	/** Wire a listener to the OS preference. Returns the unsubscribe
	 *  fn so a `+layout.svelte` `$effect` can clean up on unmount. */
	bindOsListener(): () => void {
		if (typeof window === 'undefined' || !window.matchMedia) return () => {};
		const mq = window.matchMedia('(prefers-color-scheme: dark)');
		const handler = (e: MediaQueryListEvent) => {
			this.osDark = e.matches;
		};
		mq.addEventListener('change', handler);
		return () => mq.removeEventListener('change', handler);
	}
}

export const theme = new ThemeStore();
export type { ThemeChoice };
