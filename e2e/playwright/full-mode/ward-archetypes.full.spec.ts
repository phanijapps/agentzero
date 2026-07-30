import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { expect } from "@playwright/test";
import { bootFullMode } from "../lib/harness-full";

const { test, handle } = bootFullMode({
  fixture: "ward-archetypes",
  freshVault: true,
});

test.describe("ward archetypes (Mode Full)", () => {
  test("fresh bootstrap creates every compact archetype bundle", async ({ page, request }) => {
    await page.goto(handle.uiUrl("/research"));
    await page
      .locator("textarea")
      .fill("Create one fresh ward for each of the seven archetypes, then confirm completion.");
    await page.locator('button[title="Send message"]').click();

    await expect(page.locator(".research-msg--assistant").first()).toContainText(
      /seven compact wards/i,
      { timeout: 20_000 },
    );

    for (const [archetype, name] of [
      ["generic", "fresh-generic"],
      ["coding", "fresh-code"],
      ["documentation", "fresh-docs"],
      ["journal", "fresh-journal"],
      ["ebook", "fresh-ebook"],
      ["research", "fresh-research"],
      ["news", "fresh-news"],
    ] as const) {
      const ward = join(handle.dataDir(), "wards", name);
      await expect.poll(() => existsSync(join(ward, "ward-conf.yaml"))).toBe(true);
      expect(existsSync(join(ward, `${name}.md`))).toBe(true);
      expect(existsSync(join(ward, "log.md"))).toBe(true);
      expect(existsSync(join(ward, "index.md"))).toBe(false);
      expect(existsSync(join(ward, ".zbot"))).toBe(false);
      expect(readFileSync(join(ward, "AGENTS.md"), "utf8").toLowerCase()).toContain(
        `${archetype} ward agent`,
      );
    }
    const coding = join(handle.dataDir(), "wards", "fresh-code");
    expect(existsSync(join(coding, "src"))).toBe(false);
    expect(existsSync(join(coding, "tests"))).toBe(false);
    expect(existsSync(join(coding, "docs"))).toBe(false);
    const catalog = readFileSync(
      join(handle.dataDir(), "wards", "index.md"),
      "utf8",
    );
    for (const name of [
      "fresh-generic",
      "fresh-code",
      "fresh-docs",
      "fresh-journal",
      "fresh-ebook",
      "fresh-research",
      "fresh-news",
    ]) {
      expect(catalog).toContain(`[[${name}/${name}|`);
    }
    expect(existsSync(join(handle.dataDir(), "data", "conversations.db"))).toBe(
      true,
    );

    await handle.assertZeroDrift(request);
  });
});
