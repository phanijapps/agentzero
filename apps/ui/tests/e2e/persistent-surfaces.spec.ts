import { expect, test } from "@playwright/test";

test("configures and clears persistent infographics from Settings", async ({ page }) => {
  let persistSurfaces = false;
  let clearRequests = 0;

  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;

    if (path === "/api/health") {
      await route.fulfill({ json: { status: "ok", version: "test" } });
      return;
    }
    if (path === "/api/commissioning/status") {
      await route.fulfill({
        json: {
          state: "complete",
          semanticProfile: {
            version: 1,
            basePackIds: [],
            domainPackIds: [],
            provisioning: "deferred",
          },
        },
      });
      return;
    }
    if (path === "/api/paths") {
      await route.fulfill({ json: { vaultDirDisplay: "~/Documents/zbot" } });
      return;
    }
    if (path === "/api/providers") {
      await route.fulfill({ json: [] });
      return;
    }
    if (path === "/api/models") {
      await route.fulfill({ json: {} });
      return;
    }
    if (path === "/api/settings/tools") {
      await route.fulfill({
        json: {
          success: true,
          data: {
            fileTools: true,
            offloadLargeResults: true,
            offloadThresholdTokens: 5000,
          },
        },
      });
      return;
    }
    if (path === "/api/settings/logs") {
      await route.fulfill({
        json: {
          success: true,
          data: {
            enabled: false,
            level: "info",
            rotation: "daily",
            maxFiles: 7,
            suppressStdout: false,
            restartRequired: true,
          },
        },
      });
      return;
    }
    if (path === "/api/settings/execution") {
      await route.fulfill({
        json: {
          success: true,
          data: {
            maxParallelAgents: 2,
            setupComplete: true,
            featureFlags: {},
            restartRequired: false,
          },
        },
      });
      return;
    }
    if (path === "/api/settings/presentation") {
      if (request.method() === "PUT") {
        persistSurfaces = (request.postDataJSON() as { persistSurfaces: boolean })
          .persistSurfaces;
      }
      await route.fulfill({
        json: {
          success: true,
          data: { persistSurfaces, restartRequired: false },
        },
      });
      return;
    }
    if (path === "/api/surfaces/saved" && request.method() === "DELETE") {
      expect(request.postDataJSON()).toEqual({
        confirmation: "clear_saved_infographics",
      });
      clearRequests += 1;
      await route.fulfill({ json: { deletedCount: 2 } });
      return;
    }

    await route.fulfill({ status: 404, json: { error: "not mocked" } });
  });

  await page.goto("/settings?tab=general");
  await expect(page.getByRole("heading", { name: "Infographics" })).toBeVisible();

  const toggle = page.getByRole("checkbox", { name: "Persist infographics" });
  await expect(toggle).not.toBeChecked();
  await expect(
    page.getByText(/Turning this off keeps previously saved infographics/i),
  ).toBeVisible();

  await toggle.check();
  await expect(toggle).toBeChecked();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();

  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toContain("Clear every saved infographic");
    await dialog.accept();
  });
  await page.getByRole("button", { name: "Clear saved infographics" }).click();

  await expect(
    page.getByText("2 saved infographics cleared. Open chats are unchanged."),
  ).toBeVisible();
  expect(clearRequests).toBe(1);
});

test("restores a saved infographic after Quick Chat and Research reloads", async ({ page }) => {
  const now = "2026-07-28T12:00:00Z";
  const surface = {
    surface_id: "persisted-callout",
    catalog_id: "zbot/work-surface/v1",
    components: [{
      id: "summary",
      type: "Callout",
      props: {
        title: "Persisted insight",
        message_path: "/message",
        tone: "success",
      },
    }],
    data: { message: "Restored after refresh" },
  };

  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;

    if (path === "/api/health") {
      await route.fulfill({ json: { status: "ok", version: "test" } });
      return;
    }
    if (path === "/api/commissioning/status") {
      await route.fulfill({
        json: {
          state: "complete",
          semanticProfile: {
            version: 1,
            basePackIds: [],
            domainPackIds: [],
            provisioning: "deferred",
          },
        },
      });
      return;
    }
    if (path === "/api/chat/init") {
      await route.fulfill({
        json: {
          sessionId: "chat-persist",
          conversationId: "chat-persist",
          created: false,
        },
      });
      return;
    }
    if (path.endsWith("/messages")) {
      const research = path.includes("research-persist");
      await route.fulfill({
        json: [{
          id: research ? "research-message" : "chat-message",
          execution_id: research ? "research-root" : "chat-root",
          agent_id: "root",
          delegation_type: "root",
          role: "user",
          content: research ? "Research this topic" : "Show a summary",
          created_at: now,
        }],
      });
      return;
    }
    if (path.endsWith("/artifacts")) {
      await route.fulfill({ json: [] });
      return;
    }
    if (path.endsWith("/surfaces")) {
      await route.fulfill({ json: [surface] });
      return;
    }
    if (path === "/api/logs/sessions") {
      await route.fulfill({
        json: [{
          session_id: "research-root",
          conversation_id: "research-persist",
          agent_id: "root",
          agent_name: "Root",
          title: "Persisted research",
          started_at: now,
          ended_at: now,
          status: "completed",
          token_count: 10,
          tool_call_count: 0,
          error_count: 0,
          child_session_ids: [],
        }],
      });
      return;
    }
    if (path === "/api/sessions/research-persist/state") {
      await route.fulfill({
        json: {
          session: {
            id: "research-persist",
            title: "Persisted research",
            status: "completed",
            startedAt: now,
            durationMs: 100,
            tokenCount: 10,
            model: null,
          },
          userMessage: "Research this topic",
          phase: "completed",
          response: null,
          intentAnalysis: null,
          ward: null,
          recalledFacts: [],
          plan: [],
          subagents: [],
          isLive: false,
        },
      });
      return;
    }

    await route.fulfill({ status: 404, json: { error: "not mocked" } });
  });

  await page.goto("/chat");
  await expect(page.getByText("Restored after refresh")).toBeVisible();
  await page.reload();
  await expect(page.getByText("Restored after refresh")).toBeVisible();
  await expect(page.getByText("Restored after refresh")).toHaveCount(1);

  await page.goto("/research/research-persist");
  await expect(page.getByText("Restored after refresh")).toBeVisible();
  await page.reload();
  await expect(page.getByText("Restored after refresh")).toBeVisible();
  await expect(page.getByText("Restored after refresh")).toHaveCount(1);
});
