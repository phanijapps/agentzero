import { test as base, expect, Page } from '@playwright/test';

/** Keep protected-route tests independent from the host vault's first-run state. */
export async function mockCommissioningComplete(page: Page) {
  await page.route('**/api/commissioning/status', async (route) => {
    await route.fulfill({
      json: {
        state: 'complete',
        semanticProfile: {
          version: 1,
          basePackIds: [],
          domainPackIds: [],
          provisioning: 'deferred',
        },
      },
    });
  });
}

/**
 * Page Object: Dashboard
 * Encapsulates dashboard page interactions for cleaner tests.
 */
export class DashboardPage {
  constructor(private page: Page) {}

  /** Navigate to the dashboard (dashboard is at root '/') */
  async goto() {
    await this.page.goto('/');
  }

  /** Wait for dashboard to fully load */
  async waitForLoad() {
    // Wait for dashboard header or page container
    await this.page.waitForSelector('h1:has-text("Dashboard"), .page-header, .page', {
      state: 'visible',
      timeout: 15_000,
    });
  }

  /** Get the count of visible sessions */
  async getSessionCount(): Promise<number> {
    const sessions = await this.page.locator('[data-testid="session-card"], .session-card').all();
    return sessions.length;
  }

  /** Filter sessions by source */
  async filterBySource(source: string) {
    const filter = this.page.locator('[data-testid="source-filter"], .source-filter');
    if (await filter.isVisible()) {
      await filter.click();
      await this.page.click(`[data-testid="source-option-${source}"], [data-value="${source}"]`);
    }
  }

  /** Get all visible session cards */
  async getVisibleSessions() {
    return this.page.locator('[data-testid="session-card"], .session-card').all();
  }

  /** Check if stats panel is visible */
  async isStatsPanelVisible(): Promise<boolean> {
    const statsPanel = this.page.locator('[data-testid="stats-panel"], .stats-panel, .stats');
    return statsPanel.isVisible();
  }

  /** Get a specific stat value by label */
  async getStatValue(label: string): Promise<string | null> {
    const stat = this.page.locator(`text=${label}`).locator('..').locator('.stat-value, [data-testid="stat-value"]');
    if (await stat.isVisible()) {
      return stat.textContent();
    }
    return null;
  }
}

/**
 * Page Object: Chat
 * Encapsulates chat page interactions.
 */
export class ChatPage {
  constructor(private page: Page) {}

  /** Navigate to chat with optional conversation ID */
  async goto(conversationId?: string) {
    if (conversationId) {
      await this.page.goto(`/chat/${conversationId}`);
    } else {
      await this.page.goto('/');
    }
  }

  /** Wait for chat to be ready */
  async waitForReady() {
    await this.page.waitForSelector('[data-testid="chat-input"], .chat-input, textarea', {
      state: 'visible',
      timeout: 10_000,
    });
  }

  /** Send a message */
  async sendMessage(message: string) {
    const input = this.page.locator('[data-testid="chat-input"], .chat-input, textarea').first();
    await input.fill(message);
    
    const sendButton = this.page.locator('[data-testid="send-button"], button[type="submit"], .send-button');
    await sendButton.click();
  }

  /** Wait for assistant response */
  async waitForResponse(timeout = 30_000): Promise<string | null> {
    const response = this.page.locator('[data-testid="assistant-message"], .assistant-message, .message-assistant').last();
    await response.waitFor({ state: 'visible', timeout });
    return response.textContent();
  }

  /** Get all messages */
  async getMessages() {
    return this.page.locator('[data-testid="message"], .message').all();
  }

  /** Check if chat is loading/thinking */
  async isLoading(): Promise<boolean> {
    const loader = this.page.locator('[data-testid="loading"], .loading, .thinking');
    return loader.isVisible();
  }
}

/**
 * Page Object: Settings
 * Encapsulates settings page interactions.
 */
export class SettingsPage {
  constructor(private page: Page) {}

  /** Navigate to settings */
  async goto() {
    await this.page.goto('/settings');
  }

  /** Navigate to a specific settings tab */
  async gotoTab(tab: 'agents' | 'providers' | 'mcps' | 'skills' | 'tools') {
    await this.page.goto(`/settings/${tab}`);
  }

  /** Wait for settings page to load */
  async waitForLoad() {
    await this.page.waitForSelector('[data-testid="settings"], .settings-page, main', {
      state: 'visible',
      timeout: 10_000,
    });
  }
}

/**
 * Page Object: Mission Control
 * Encapsulates Mission Control page interactions.
 */
export class MissionControlPage {
  constructor(private page: Page) {}

  async goto() {
    await this.page.goto('/mission-control');
    await this.page.waitForSelector('.kpi-strip', { state: 'visible', timeout: 15_000 });
    // Wait until the session list has settled: either rows appear OR the list
    // shows a non-loading empty state ("No sessions match these filters.").
    // We can't just wait for the "Loading" text to disappear because it may
    // not have appeared yet when this check runs.
    await this.page.waitForFunction(
      () => {
        const rows = document.querySelectorAll('.session-list-panel__row');
        if (rows.length > 0) return true;
        const empties = document.querySelectorAll('.session-list-panel__empty');
        for (const el of empties) {
          if (el.textContent && !el.textContent.includes('Loading')) return true;
        }
        return false;
      },
      { timeout: 15_000 },
    );
  }

  /** Find and click a session row by partial title text. Returns false if not found. */
  async selectSession(titleFragment: string): Promise<boolean> {
    const row = this.page.locator('.session-list-panel__row', { hasText: titleFragment }).first();
    if (!(await row.isVisible().catch(() => false))) return false;
    await row.click();
    await this.page.waitForSelector('.session-detail-pane:not(.session-detail-pane--empty)', {
      state: 'visible',
      timeout: 10_000,
    });
    return true;
  }

  /** Find any session row that has multiple subagents (shows N↳ in its meta). */
  async selectFirstMultiSubagentSession(): Promise<boolean> {
    // Rows with 2+ subagents show "N ↳" in the meta span
    // Match "N ↳" or "N↳" (space may be collapsed in the accessible name)
    const rows = this.page.locator('.session-list-panel__row').filter({ hasText: /\d+ ?↳/ });
    const count = await rows.count();
    if (count === 0) return false;
    await rows.first().click();
    await this.page.waitForSelector('.session-detail-pane:not(.session-detail-pane--empty)', {
      state: 'visible',
      timeout: 10_000,
    });
    return true;
  }

  /** Wait for the tools pane trace tree to finish loading. */
  async waitForTrace() {
    await this.page.waitForSelector('.agent-tool-tree', { state: 'visible', timeout: 15_000 });
  }

  /** All depth-1 subagent group headers (direct delegations of root). */
  subagentTokens() {
    return this.page.locator('.agent-tool-group--depth-1 .agent-tool-group__tokens');
  }

  /** Toggle a status filter chip (RUNNING, QUEUED, DONE, FAILED, PAUSED). */
  async toggleFilter(label: string) {
    await this.page.getByRole('button', { name: label }).click();
  }

  kpiStrip() {
    return this.page.locator('.kpi-strip');
  }

  sessionList() {
    return this.page.locator('.session-list-panel__list');
  }

  detailPane() {
    return this.page.locator('.session-detail-pane:not(.session-detail-pane--empty)');
  }

  searchBox() {
    return this.page.locator('.session-list-panel__search');
  }
}

// Extend base test with custom fixtures
export const test = base.extend<{
  dashboardPage: DashboardPage;
  chatPage: ChatPage;
  settingsPage: SettingsPage;
  mcPage: MissionControlPage;
}>({
  dashboardPage: async ({ page }, use) => {
    const dashboard = new DashboardPage(page);
    await use(dashboard);
  },
  chatPage: async ({ page }, use) => {
    const chat = new ChatPage(page);
    await use(chat);
  },
  settingsPage: async ({ page }, use) => {
    const settings = new SettingsPage(page);
    await use(settings);
  },
  mcPage: async ({ page }, use) => {
    await use(new MissionControlPage(page));
  },
});

// Re-export expect for convenience
export { expect };
