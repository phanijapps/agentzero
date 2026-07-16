import { expect } from "@playwright/test";
import { bootFullMode } from "../lib/harness-full";

const { test, handle } = bootFullMode({ fixture: "unified-recall" });

test.describe("unified-recall (Mode Full)", () => {
  test("the acceptance prompt exposes the normal recall tool call", async ({ page, request }) => {
    await page.goto(handle.uiUrl("/research"));

    await page.locator("textarea").fill(
      "Before answering, call recall for 'knowledge graph memory' and list the source types you found.",
    );
    await page.locator('button[title="Send message"]').click();

    await expect.poll(() => page.url(), { timeout: 10_000 })
      .toMatch(/\/research\/sess-/);

    const answer = page.locator(".research-msg--assistant").first();
    await expect(answer).toContainText(/mode: unified/i, { timeout: 20_000 });
    await expect(answer).toContainText(/facts, knowledge graph, ward wiki/i);

    // The live status pill announces `recall` while the turn is running.
    // Completed turns retain a compact, argument-free activity row so the
    // model-visible tool invocation remains inspectable after the stream.
    await expect(page.getByTestId("turn-tool-activity"))
      .toContainText("recall", { timeout: 20_000 });

    await handle.assertZeroDrift(request);
  });
});
