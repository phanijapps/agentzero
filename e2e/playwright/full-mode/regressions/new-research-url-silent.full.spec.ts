import { expect } from "@playwright/test";
import { bootFullMode } from "../../lib/harness-full";

const { test, handle } = bootFullMode({ fixture: "simple-qa" });

test.describe("regression: URL silent after New Research (Mode Full)", () => {
  test("real zerod must emit invoke_accepted for the new session so UI's URL flips", async ({ page }) => {
    await page.goto(handle.uiUrl("/research"));

    await page.locator("textarea").fill("what is 2+2? one-line answer");
    await page.locator('button[title="Send message"]').click();
    await expect.poll(() => page.url(), { timeout: 10_000 })
      .toMatch(/\/research\/sess-/);
    await expect(page.locator(".research-msg--assistant").first())
      .toBeVisible({ timeout: 20_000 });

    await page.getByRole("button", { name: /New research/i }).click();
    await expect.poll(() => page.url()).toMatch(/\/research$/);

    await page.locator("textarea").fill("second prompt");
    await page.locator('button[title="Send message"]').click();
    await expect.poll(() => page.url(), { timeout: 5_000 })
      .toMatch(/\/research\/sess-/);
  });
});
