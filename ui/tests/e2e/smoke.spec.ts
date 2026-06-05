import { expect, test } from '@playwright/test';

/**
 * Dashboard smoke. Walks the dashboard from a factory-fresh boot
 * to a working workspace + app + collection:
 *
 *  1. visit `/_/setup` (the public route reachable before the
 *     server is initialized)
 *  2. complete the wizard (set the master password)
 *  3. the wizard auto-logs us in and bounces to `/_/workspaces`
 *  4. create a workspace
 *  5. drill in, create an app
 *  6. drill in, create a `base` collection
 *
 * Run sequentially against a single shared backend; every assertion
 * abort stops the suite. The harness drops state between full-suite
 * runs by deleting the temp `data/` directory on teardown, so the
 * test always starts from "uninitialized server".
 */

const MASTER_PW = 'hunter22-e2e';
const WORKSPACE_ID = 'acme';
const APP_ID = 'mobile';
const COLLECTION_ID = 'notes';

test.describe.serial('dashboard smoke', () => {
	test.setTimeout(120_000);

	test('factory → workspace → app → collection', async ({ page }) => {
		// 1–2. Setup wizard.
		await page.goto('/_/setup');
		await expect(page.getByRole('heading', { name: 'Set up RustBase' })).toBeVisible();
		await page.locator('#password').fill(MASTER_PW);
		await page.getByRole('button', { name: 'Set master password' }).click();

		// 3. Setup auto-logs us in and bounces to `/_/workspaces`.
		await page.waitForURL(/\/_\/workspaces$/);
		await expect(page.getByRole('heading', { name: 'Workspaces' })).toBeVisible();

		// 4. Create a workspace.
		await page.getByRole('button', { name: /New workspace/ }).click();
		await page.locator('#id').fill(WORKSPACE_ID);
		await page.locator('#name').fill('Acme E2E');
		await page.getByRole('button', { name: /^Create$/ }).click();

		// The new row shows up in the table.
		const workspaceRow = page.getByRole('row').filter({ hasText: WORKSPACE_ID });
		await expect(workspaceRow).toBeVisible();

		// 5. Drill in, create an app.
		await workspaceRow.click();
		await page.waitForURL(new RegExp(`/_/workspaces/${WORKSPACE_ID}$`));
		await expect(
			page.getByRole('heading', { name: new RegExp(`Workspace\\s+${WORKSPACE_ID}`) })
		).toBeVisible();

		await page.getByRole('button', { name: /New app/ }).click();
		await page.locator('#id').fill(APP_ID);
		await page.locator('#name').fill('Mobile');
		await page.getByRole('button', { name: /^Create$/ }).click();

		const appRow = page.getByRole('row').filter({ hasText: APP_ID });
		await expect(appRow).toBeVisible();

		// 6. Drill in, create a base collection. The create flow drops
		//    the user straight into the schema editor on success; we
		//    assert we ended up on the new collection's page.
		await appRow.click();
		await page.waitForURL(new RegExp(`/_/workspaces/${WORKSPACE_ID}/apps/${APP_ID}$`));
		await expect(
			page.getByRole('heading', { name: new RegExp(`App\\s+${APP_ID}`) })
		).toBeVisible();

		await page.getByRole('button', { name: /New collection/ }).click();
		await page.locator('#id').fill(COLLECTION_ID);
		await page.getByRole('button', { name: /^Create$/ }).click();

		await page.waitForURL(
			new RegExp(`/_/workspaces/${WORKSPACE_ID}/apps/${APP_ID}/collections/${COLLECTION_ID}$`)
		);
	});
});
