import { defineConfig, devices } from '@playwright/test';

/**
 * End-to-end smoke tests for the embedded dashboard.
 *
 * Backend: a release build of `rustbase` is booted by
 * `scripts/e2e-server.sh` against a throw-away `data/` directory.
 * The dashboard is served from the binary's embedded copy
 * (`RUSTBASE_DASHBOARD_PATH` not set), so the same artifact you
 * ship to prod is what's under test.
 *
 * Tests run sequentially against a single shared backend
 * (`workers: 1`) — every spec assumes a fresh data/ tree, so the
 * harness drops state between runs by deleting the data dir on
 * teardown. Running individual specs concurrently would race on
 * the master-admin seed row.
 */
export default defineConfig({
	testDir: './tests/e2e',
	fullyParallel: false,
	workers: 1,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 2 : 0,
	reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
	use: {
		baseURL: 'http://127.0.0.1:8989',
		trace: 'retain-on-failure',
		screenshot: 'only-on-failure'
	},
	webServer: {
		command: 'bash ../scripts/e2e-server.sh',
		url: 'http://127.0.0.1:8989/healthz',
		reuseExistingServer: !process.env.CI,
		timeout: 180_000,
		stdout: 'pipe',
		stderr: 'pipe'
	},
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'] }
		}
	]
});
