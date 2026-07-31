import { test, expect } from './fixtures';

/**
 * Smoke tests - verify basic app functionality.
 * These tests should always pass and run quickly.
 */
test.describe('Smoke Tests', () => {
  test('app loads successfully', async ({ page }) => {
    await page.goto('/');

    // Should have a title
    await expect(page).toHaveTitle(/z-Bot/i);
  });

  test('navigation works', async ({ page }) => {
    await page.goto('/');

    // Page should be visible
    await expect(page.locator('body')).toBeVisible();
  });

  test('can navigate to dashboard', async ({ page }) => {
    // Root intentionally redirects to the primary research workspace.
    await page.goto('/');

    await expect(page).toHaveURL(/\/research$/);
  });

  test('can navigate to settings', async ({ page }) => {
    await page.goto('/settings');

    // Should navigate successfully
    await expect(page).toHaveURL(/settings/);
  });
});

test.describe('Research Page', () => {
  test('root loads the research workspace', async ({ dashboardPage, page }) => {
    await dashboardPage.goto();

    await expect(page).toHaveURL(/\/research$/);
  });
});

test.describe('Chat Page', () => {
  test('chat page loads', async ({ chatPage, page }) => {
    await chatPage.goto();

    // Should load successfully
    await expect(page.locator('body')).toBeVisible();
  });
});
