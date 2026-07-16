import { expect } from "@playwright/test";
import { bootFullMode } from "../lib/harness-full";

const { test, handle } = bootFullMode({ fixture: "simple-qa" });

test.describe("simple-qa (Mode Full)", () => {
  test("real zerod + mock-llm replays the single-turn respond", async ({ page, request }) => {
    await page.goto(handle.uiUrl("/research"));

    await page.locator("textarea").fill("what is 2+2? one-line answer");
    await page.locator('button[title="Send message"]').click();

    await expect.poll(() => page.url(), { timeout: 10_000 })
      .toMatch(/\/research\/sess-/);

    await expect(page.locator(".research-msg--assistant").first())
      .toContainText(/4/, { timeout: 20_000 });

    await handle.assertZeroDrift(request);
  });
});
