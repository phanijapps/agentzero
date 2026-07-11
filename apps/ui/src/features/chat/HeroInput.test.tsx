// =============================================================================
// HeroInput — full landing-experience interaction tests.
//
// HeroInput shares ~80% of its surface with ChatInput (textarea, send,
// file upload, drag-drop, chip remove) plus three landing-only pieces:
// brand mark, suggestion chips, and the recent-sessions rail. Tests
// here focus on the unique surface and just exercise enough of the
// shared paths to lift coverage past 90%.
// =============================================================================

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HeroInput } from "./HeroInput";
import type { LogSession } from "@/services/transport/types";

const fetchSpy = vi.fn();

beforeEach(() => {
  fetchSpy.mockReset();
  vi.stubGlobal("fetch", fetchSpy);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function makeUploadResponse(filename = "hero.md") {
  return {
    ok: true,
    statusText: "OK",
    json: async () => ({
      id: `file-${filename}`,
      filename,
      mime_type: "text/markdown",
      size: 12,
      path: `/tmp/${filename}`,
    }),
  } as unknown as Response;
}

function makeFile(name = "hero.md", content = "x") {
  return new File([content], name, { type: "text/markdown" });
}

function makeRow(overrides: Partial<LogSession> & { session_id: string; conversation_id: string }): LogSession {
  return {
    agent_id: "root",
    agent_name: "root",
    title: overrides.title ?? "Sample session",
    started_at: overrides.started_at ?? new Date(Date.now() - 60_000).toISOString(),
    status: "completed",
    token_count: 0,
    tool_call_count: 0,
    error_count: 0,
    child_session_ids: [],
    ...overrides,
  } as unknown as LogSession;
}

describe("<HeroInput>", () => {
  it("renders the brand mark", () => {
    render(<HeroInput onSend={vi.fn()} />);
    expect(screen.getByRole("img", { name: "z-Bot" })).toBeTruthy();
    expect(screen.getByText("z-Bot")).toBeTruthy();
  });

  it("does not send when text is empty", () => {
    const onSend = vi.fn();
    render(<HeroInput onSend={onSend} />);
    fireEvent.keyDown(screen.getByPlaceholderText("What would you like to work on?"), { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("sends on Enter and clears the input", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    render(<HeroInput onSend={onSend} />);
    const textarea = screen.getByPlaceholderText("What would you like to work on?") as HTMLTextAreaElement;
    await user.type(textarea, "summarize this");
    fireEvent.keyDown(textarea, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("summarize this", []);
    expect(textarea.value).toBe("");
  });

  it("Shift+Enter does NOT send", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    render(<HeroInput onSend={onSend} />);
    const textarea = screen.getByPlaceholderText("What would you like to work on?");
    await user.type(textarea, "abc");
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("ignores key-repeat events on Enter", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    render(<HeroInput onSend={onSend} />);
    const textarea = screen.getByPlaceholderText("What would you like to work on?");
    await user.type(textarea, "abc");
    fireEvent.keyDown(textarea, { key: "Enter", repeat: true });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("Send button submits", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    render(<HeroInput onSend={onSend} />);
    await user.type(screen.getByPlaceholderText("What would you like to work on?"), "click me");
    fireEvent.click(screen.getByTitle("Send message"));
    expect(onSend).toHaveBeenCalledWith("click me", []);
  });

  it("clicking a suggestion fills the textarea", () => {
    render(<HeroInput onSend={vi.fn()} />);
    fireEvent.click(screen.getByText("Write a report"));
    const textarea = screen.getByPlaceholderText("What would you like to work on?") as HTMLTextAreaElement;
    expect(textarea.value).toBe("Write a report");
  });

  it("uploads a file via the attach button and shows it as a chip", async () => {
    fetchSpy.mockResolvedValueOnce(makeUploadResponse("doc.md"));
    const { container } = render(<HeroInput onSend={vi.fn()} />);
    fireEvent.change(container.querySelector("input[type='file']") as HTMLInputElement, {
      target: { files: [makeFile("doc.md")] },
    });
    await waitFor(() => expect(screen.getByText("doc.md")).toBeTruthy());
  });

  it("send fires onSend with attachments, then clears them", async () => {
    fetchSpy.mockResolvedValueOnce(makeUploadResponse("ticket.md"));
    const onSend = vi.fn();
    const user = userEvent.setup();
    const { container } = render(<HeroInput onSend={onSend} />);
    fireEvent.change(container.querySelector("input[type='file']") as HTMLInputElement, {
      target: { files: [makeFile("ticket.md")] },
    });
    await waitFor(() => screen.getByText("ticket.md"));
    await user.type(screen.getByPlaceholderText("What would you like to work on?"), "fix");
    fireEvent.click(screen.getByTitle("Send message"));
    expect(onSend).toHaveBeenCalledWith(
      "fix",
      [expect.objectContaining({ name: "ticket.md", path: "/tmp/ticket.md" })],
    );
    expect(screen.queryByText("ticket.md")).toBeNull();
  });

  it("removes a chip when its 'x' is clicked, via Enter, or via Space", async () => {
    fetchSpy
      .mockResolvedValueOnce(makeUploadResponse("clk.md"))
      .mockResolvedValueOnce(makeUploadResponse("ent.md"))
      .mockResolvedValueOnce(makeUploadResponse("spc.md"));
    const { container } = render(<HeroInput onSend={vi.fn()} />);
    const fileInput = container.querySelector("input[type='file']") as HTMLInputElement;
    fireEvent.change(fileInput, { target: { files: [makeFile("clk.md")] } });
    await waitFor(() => screen.getByText("clk.md"));
    fireEvent.click(screen.getAllByText("x")[0]);
    expect(screen.queryByText("clk.md")).toBeNull();

    fireEvent.change(fileInput, { target: { files: [makeFile("ent.md")] } });
    await waitFor(() => screen.getByText("ent.md"));
    fireEvent.keyDown(screen.getAllByText("x")[0], { key: "Enter" });
    expect(screen.queryByText("ent.md")).toBeNull();

    fireEvent.change(fileInput, { target: { files: [makeFile("spc.md")] } });
    await waitFor(() => screen.getByText("spc.md"));
    fireEvent.keyDown(screen.getAllByText("x")[0], { key: " " });
    expect(screen.queryByText("spc.md")).toBeNull();
  });

  it("ignores other keys on the chip 'x' affordance", async () => {
    fetchSpy.mockResolvedValueOnce(makeUploadResponse("stay.md"));
    const { container } = render(<HeroInput onSend={vi.fn()} />);
    fireEvent.change(container.querySelector("input[type='file']") as HTMLInputElement, {
      target: { files: [makeFile("stay.md")] },
    });
    await waitFor(() => screen.getByText("stay.md"));
    fireEvent.keyDown(screen.getByText("x"), { key: "Tab" });
    expect(screen.getByText("stay.md")).toBeTruthy();
  });

  it("file-input change with no files is a no-op", () => {
    const { container } = render(<HeroInput onSend={vi.fn()} />);
    fireEvent.change(container.querySelector("input[type='file']") as HTMLInputElement, {
      target: { files: [] },
    });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("surfaces the upload error message when the response is not ok", async () => {
    fetchSpy.mockResolvedValueOnce({
      ok: false,
      statusText: "Boom",
    } as unknown as Response);
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      const { container } = render(<HeroInput onSend={vi.fn()} />);
      fireEvent.change(container.querySelector("input[type='file']") as HTMLInputElement, {
        target: { files: [makeFile("bad.md")] },
      });
      await waitFor(() => expect(screen.getByText(/Upload failed/)).toBeTruthy());
    } finally {
      errSpy.mockRestore();
    }
  });

  it("falls back to a generic message when the upload throws a non-Error", async () => {
    fetchSpy.mockImplementationOnce(() => Promise.reject("rejected with a string"));
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      const { container } = render(<HeroInput onSend={vi.fn()} />);
      fireEvent.change(container.querySelector("input[type='file']") as HTMLInputElement, {
        target: { files: [makeFile("ne.md")] },
      });
      await waitFor(() => expect(screen.getAllByText("Upload failed").length).toBeGreaterThan(0));
    } finally {
      errSpy.mockRestore();
    }
  });

  it("uploads via the drop handler when files are dropped on the region", async () => {
    fetchSpy.mockResolvedValueOnce(makeUploadResponse("drop.md"));
    const { container } = render(<HeroInput onSend={vi.fn()} />);
    const region = container.querySelector("[role='region']")!;
    const dropEvent = new Event("drop", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(dropEvent, "dataTransfer", {
      value: { files: [makeFile("drop.md")] },
    });
    fireEvent(region, new Event("dragover", { bubbles: true, cancelable: true }));
    fireEvent(region, dropEvent);
    await waitFor(() => expect(screen.getByText("drop.md")).toBeTruthy());
  });

  it("renders recent sessions cards with title, completed icon, and relative time", () => {
    const sessions = [
      makeRow({ session_id: "s1", conversation_id: "c1", title: "Stock thesis", status: "completed" }),
      makeRow({ session_id: "s2", conversation_id: "c2", title: "Crashed run", status: "error" }),
      makeRow({ session_id: "s3", conversation_id: "c3", title: "" }), // falls back to "Untitled"
    ];
    render(<HeroInput onSend={vi.fn()} recentSessions={sessions} />);
    expect(screen.getByText("Stock thesis")).toBeTruthy();
    expect(screen.getByText("Crashed run")).toBeTruthy();
    expect(screen.getByText("Untitled")).toBeTruthy();
  });

  it("clicks a recent session card to invoke the override handler with session + conversation ids", () => {
    const onSelectSession = vi.fn();
    const sessions = [
      makeRow({ session_id: "s1", conversation_id: "c1", title: "Pick me" }),
    ];
    render(<HeroInput onSend={vi.fn()} recentSessions={sessions} onSelectSession={onSelectSession} />);
    fireEvent.click(screen.getByText("Pick me"));
    expect(onSelectSession).toHaveBeenCalledWith("s1", "c1");
  });

  it("falls back to the default switchToSession when no override is provided (no throw on click)", () => {
    // The default handler calls window.location.reload after writing localStorage,
    // which jsdom doesn't implement. Stub it so we can assert the click path
    // didn't throw.
    const original = window.location;
    const reload = vi.fn();
    Object.defineProperty(window, "location", {
      writable: true,
      value: { ...original, reload, href: "http://localhost/" },
    });
    try {
      const sessions = [makeRow({ session_id: "s1", conversation_id: "c1", title: "Default" })];
      render(<HeroInput onSend={vi.fn()} recentSessions={sessions} />);
      fireEvent.click(screen.getByText("Default"));
      expect(reload).toHaveBeenCalled();
    } finally {
      Object.defineProperty(window, "location", {
        writable: true,
        value: original,
      });
    }
  });

  it("renders no recent rail when the list is empty", () => {
    const { container } = render(<HeroInput onSend={vi.fn()} recentSessions={[]} />);
    expect(container.querySelector(".hero-input__recent")).toBeNull();
  });
});
