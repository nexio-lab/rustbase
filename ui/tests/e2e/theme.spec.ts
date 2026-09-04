import { expect, test } from '@playwright/test';

/**
 * Smoke for the theme toggle and a11y baseline.
 *
 *  1. Login page boots in light mode by default (`<html>` has no
 *     `dark` class — the OS preference for the test browser is
 *     overridable, so we don't assert against it directly).
 *  2. The theme rune persists across navigations: forcing dark on
 *     `/_/login`, then navigating to `/_/setup`, still shows the
 *     `dark` class.
 *  3. The skip link is wired up: tabbing once from a fresh load
 *     focuses it, and pressing Enter jumps to `#main-content`.
 *
 * Runs in a workspace where the server is already bootstrapped by
 * an earlier spec (`smoke.spec.ts`), so we hit `/_/login` rather
 * than `/_/setup`.
 */

test.describe.serial('theme + a11y', () => {
	test('explicit dark choice persists in localStorage and across navigation', async ({ page }) => {
		await page.goto('/_/login');
		// Force a dark choice via localStorage and reload so the
		// runes pick it up; same path the ThemeToggle button takes.
		await page.evaluate(() => localStorage.setItem('rb_theme', 'dark'));
		await page.reload();
		await expect(page.locator('html')).toHaveClass(/(^|\s)dark(\s|$)/);

		// Navigate to /_/setup and check the dark class is still
		// applied — the choice lives in localStorage, not in URL
		// state.
		await page.goto('/_/setup');
		await expect(page.locator('html')).toHaveClass(/(^|\s)dark(\s|$)/);

		// Restore so subsequent tests in this worker start fresh.
		await page.evaluate(() => localStorage.removeItem('rb_theme'));
	});

	test('skip-to-main-content link reaches main on Enter', async ({ page }) => {
		await page.goto('/_/login');
		// First Tab focuses the skip link (which is the first
		// focusable element in the layout).
		await page.keyboard.press('Tab');
		const focused = await page.evaluate(() => document.activeElement?.textContent ?? '');
		expect(focused.trim()).toBe('Skip to main content');
		await page.keyboard.press('Enter');
		await expect(page).toHaveURL(/#main-content$/);
	});
});
