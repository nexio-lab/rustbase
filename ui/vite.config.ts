import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// In `bun run dev` the SvelteKit dev server runs on :5173 while the
// Rust API runs on :8080. The proxy below forwards every server-side
// path to the Rust server so the dashboard can hit them with relative
// URLs, identical to production where everything lives on one origin.
const RUST = 'http://localhost:8080';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		proxy: {
			'/api': { target: RUST, changeOrigin: true },
			'/_/setup': { target: RUST, changeOrigin: true },
			'/_/auth': { target: RUST, changeOrigin: true },
			'/healthz': { target: RUST, changeOrigin: true }
		}
	}
});
