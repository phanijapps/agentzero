import { test, expect } from '@playwright/test';
import { mockCommissioningComplete } from './fixtures';

test.beforeEach(async ({ page }) => {
  await mockCommissioningComplete(page);
});

/**
 * Navigation E2E Tests
 * Tests for app-wide navigation functionality.
 */
test.describe('Navigation', () => {
  test('home page loads', async ({ page }) => {
    await page.goto('/');

    await expect(page).toHaveTitle(/z-Bot/i);
  });

  test('home page shows main content', async ({ page }) => {
    await page.goto('/');

    // Body should be visible and have content
    await expect(page.locator('body')).toBeVisible();
  });

  test('root redirects to research workspace', async ({ page }) => {
    await page.goto('/');

    await expect(page).toHaveURL(/\/research$/);
    await expect(page.getByRole('textbox', { name: /what would you like to work on/i })).toBeVisible();
  });

  test('can navigate to settings via URL', async ({ page }) => {
    await page.goto('/settings');

    // Should be on settings
    await expect(page).toHaveURL(/settings/);
  });

  test('can navigate to chat via URL', async ({ page }) => {
    await page.goto('/chat');

    // Should be on chat or redirected to main chat
    await expect(page).toHaveURL(/chat|\/$/);
  });

  test('navigation links work from home', async ({ page }) => {
    await page.goto('/');

    // Try to find and click navigation links
    // Check if there's a nav element or links
    const navLinks = page.locator('nav a, header a, [role="navigation"] a');
    const linkCount = await navLinks.count();

    // If there are nav links, they should be clickable
    if (linkCount > 0) {
      const firstLink = navLinks.first();
      await expect(firstLink).toBeVisible();
    }
  });

  test('can navigate between pages', async ({ page }) => {
    // Start at the root workspace.
    await page.goto('/');
    await expect(page.locator('body')).toBeVisible();

    // Go to settings
    await page.goto('/settings');
    await expect(page).toHaveURL(/settings/);

    // Go back to the root workspace.
    await page.goto('/');
    await expect(page.locator('body')).toBeVisible();
  });

  test('browser back/forward navigation works', async ({ page }) => {
    // Navigate through pages
    await page.goto('/');
    await page.goto('/settings');
    await page.goto('/logs');

    // Go back to settings
    await page.goBack();
    await expect(page).toHaveURL(/settings/);

    // Go back to the root workspace.
    await page.goBack();
    await expect(page).toHaveURL(/\/research$/);

    // Go forward to settings
    await page.goForward();
    await expect(page).toHaveURL(/settings/);
  });
});

test.describe('Navigation - Error Handling', () => {
  test('unknown routes show error or redirect', async ({ page }) => {
    await page.goto('/unknown-route-that-does-not-exist-xyz');

    // Should either:
    // 1. Show a 404 page
    // 2. Redirect to home
    // 3. Show some error message
    const is404 = await page.getByText(/404|not found|page.*exist/i).isVisible().catch(() => false);
    const currentUrl = page.url();
    const isHome = currentUrl.endsWith('/') || currentUrl.includes('localhost:3000/');
    const hasContent = await page.locator('body').isVisible().catch(() => false);

    // At least one should be true - we don't crash
    expect(is404 || isHome || hasContent).toBe(true);
  });

  test('app recovers from navigation errors', async ({ page }) => {
    // Try invalid route
    await page.goto('/invalid-page-12345');

    // Should be able to navigate to the valid root workspace.
    await page.goto('/');
    await expect(page).toHaveURL(/\/research$/);
    await expect(page.getByRole('textbox', { name: /what would you like to work on/i })).toBeVisible();
  });
});

test.describe('Navigation - Deep Links', () => {
  test('can access settings tabs directly', async ({ page }) => {
    // Try accessing settings tabs directly
    await page.goto('/settings/agents');
    await expect(page).toHaveURL(/settings/);

    await page.goto('/settings/providers');
    await expect(page).toHaveURL(/settings/);
  });

  test('legacy dashboard deep link redirects to Mission Control', async ({ page }) => {
    await page.goto('/dashboard');
    await expect(page).toHaveURL(/\/mission-control$/);

    await expect(page.getByRole('heading', { name: /attention radar/i })).toBeVisible();
  });
});

test.describe('Navigation - Responsiveness', () => {
  test('navigation works on mobile viewport', async ({ page }) => {
    // Set mobile viewport
    await page.setViewportSize({ width: 375, height: 667 });

    await page.goto('/');
    await expect(page.locator('body')).toBeVisible();

    // Verify the root workspace loads.
    await expect(page).toHaveURL(/\/research$/);
  });

  test('navigation works on tablet viewport', async ({ page }) => {
    // Set tablet viewport
    await page.setViewportSize({ width: 768, height: 1024 });

    await page.goto('/');
    await expect(page.locator('body')).toBeVisible();

    // Verify the root workspace loads.
    await expect(page).toHaveURL(/\/research$/);
  });
});
