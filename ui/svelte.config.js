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
		}
	}
};

export default config;
