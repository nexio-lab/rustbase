import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	compilerOptions: {
		runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true)
	},
	kit: {
		// SPA mode: every unknown route falls back to `index.html`. The
		// Rust server embeds the build output via include_dir! and
		// serves it at `/_/`. No SSR — the dashboard is purely client-
		// rendered against the existing API.
		adapter: adapter({
			fallback: 'index.html',
			pages: 'build',
			assets: 'build',
			strict: false
		}),
		// The dashboard lives at /_/ when embedded; in `bun run dev`
		// it serves at / for fast iteration. Set VITE_BASE=/_ to test
		// the embedded path locally.
		paths: {
			base: process.env.VITE_BASE ?? '',
			relative: false
		},
		// Content-Security-Policy in `hash` mode: SvelteKit computes a
		// SHA-256 of every inline boot script it emits and renders the
		// directive as a `<meta http-equiv="Content-Security-Policy">`
		// inside the page itself. That self-protects the dashboard
		// regardless of which CSP the upstream HTTP layer sends — and
		// it sidesteps the chicken-and-egg of needing a per-build
		// script hash baked into the server-side CSP header.
		csp: {
			mode: 'hash',
			directives: {
				'default-src': ['self'],
				'img-src': ['self', 'data:'],
				'style-src': ['self', 'unsafe-inline'],
				'script-src': ['self'],
				'connect-src': ['self'],
				'frame-ancestors': ['none'],
				'base-uri': ['self'],
				'form-action': ['self']
			}
		}
	}
};

export default config;
