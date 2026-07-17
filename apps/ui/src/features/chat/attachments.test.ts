import { describe, expect, it } from "vitest";
import {
  composeMessageWithAttachments,
  displayAttachments,
  splitMessageAttachments,
} from "./attachments";
import type { UploadedFile } from "./ChatInput";

const upload = (overrides: Partial<UploadedFile> = {}): UploadedFile => ({
  id: "u-1",
  name: "notes.md",
  mimeType: "text/markdown",
  size: 512,
  path: "/vault/temp/attachments/u-1.md",
  ...overrides,
});

describe("composeMessageWithAttachments", () => {
  it("returns the trimmed text unchanged when there are no attachments", () => {
    expect(composeMessageWithAttachments("  hello  ", [])).toBe("hello");
  });

  it("splits persisted attachment metadata into safe display chips", () => {
    const message = composeMessageWithAttachments("analyze this", [upload({
      name: "interview.txt",
      path: "/private/vault/temp/interview.txt",
    })]);
    expect(splitMessageAttachments(message)).toEqual({
      content: "analyze this",
      attachments: [{ name: "interview.txt", mimeType: "text/markdown", sizeLabel: "512 B" }],
    });
    expect(displayAttachments([upload({ name: "notes.md", size: 42 })])).toEqual([
      { name: "notes.md", mimeType: "text/markdown", sizeLabel: "42 B" },
    ]);
  });

  it("splices the absolute server path into the prompt so the agent can read it", () => {
    const out = composeMessageWithAttachments("summarise this", [upload()]);
    // The whole point of the fix: the path must be in the prompt text.
    expect(out).toContain("/vault/temp/attachments/u-1.md");
    expect(out).toContain("**Attached files:**");
    expect(out).toContain("notes.md");
    expect(out).toContain("text/markdown");
  });

  it("formats sub-1KB sizes in bytes, KB up to 1MB, MB above that", () => {
    const out = composeMessageWithAttachments("x", [
      upload({ id: "a", size: 999 }),
      upload({ id: "b", size: 2048 }),
      upload({ id: "c", size: 5 * 1024 * 1024 }),
    ]);
    expect(out).toContain("999 B");
    expect(out).toContain("2.0 KB");
    expect(out).toContain("5.0 MB");
  });

  it("emits one row per attachment in the markdown table", () => {
    const out = composeMessageWithAttachments("multi", [
      upload({ id: "one", name: "one.txt", path: "/tmp/one.txt" }),
      upload({ id: "two", name: "two.txt", path: "/tmp/two.txt" }),
    ]);
    expect(out).toContain("| one.txt |");
    expect(out).toContain("| two.txt |");
    expect(out).toContain("/tmp/one.txt");
    expect(out).toContain("/tmp/two.txt");
  });
});
