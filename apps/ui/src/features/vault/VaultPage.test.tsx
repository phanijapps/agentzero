import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@/test/utils";
import userEvent from "@testing-library/user-event";
import { VaultPage } from "./VaultPage";

const listVaultWards = vi.fn();
const getVaultTree = vi.fn();
const searchVaultFiles = vi.fn();
const getVaultFile = vi.fn();
const openWard = vi.fn();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({
    listVaultWards,
    getVaultTree,
    searchVaultFiles,
    getVaultFile,
    openWard,
  }),
}));

describe("VaultPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.pushState({}, "", "/");
    listVaultWards.mockResolvedValue({
      success: true,
      data: {
        wards: [
          { id: "stock-analysis", name: "stock-analysis" },
          { id: "research-lab", name: "research-lab" },
        ],
      },
    });
    getVaultTree.mockImplementation((_wardId: string, path = "") => {
      if (path === "reports") {
        return Promise.resolve({
          success: true,
          data: {
            ward_id: "stock-analysis",
            path,
            truncated: false,
            children: [
              {
                ward_id: "stock-analysis",
                path: "reports/valuation.md",
                name: "valuation.md",
                kind: "file",
                extension: "md",
                size: 11,
                previewable: true,
              },
              {
                ward_id: "stock-analysis",
                path: "reports/notes.txt",
                name: "notes.txt",
                kind: "file",
                extension: "txt",
                size: 10,
                previewable: true,
              },
              {
                ward_id: "stock-analysis",
                path: "reports/index.html",
                name: "index.html",
                kind: "file",
                extension: "html",
                size: 25,
                previewable: true,
              },
              {
                ward_id: "stock-analysis",
                path: "reports/app.py",
                name: "app.py",
                kind: "file",
                extension: "py",
                size: 18,
                previewable: true,
              },
              {
                ward_id: "stock-analysis",
                path: "reports/proposal.docx",
                name: "proposal.docx",
                kind: "file",
                extension: "docx",
                size: 512,
                previewable: true,
              },
              {
                ward_id: "stock-analysis",
                path: "reports/huge.docx",
                name: "huge.docx",
                kind: "file",
                extension: "docx",
                size: 512,
                previewable: true,
              },
              {
                ward_id: "stock-analysis",
                path: "reports/large.txt",
                name: "large.txt",
                kind: "file",
                extension: "txt",
                size: 2_097_153,
                previewable: true,
              },
            ],
          },
        });
      }
      return Promise.resolve({
        success: true,
        data: {
          ward_id: "stock-analysis",
          path: "",
          truncated: true,
          children: [
            {
              ward_id: "stock-analysis",
              path: "reports",
              name: "reports",
              kind: "directory",
              previewable: false,
            },
            {
              ward_id: "stock-analysis",
              path: "deck.ppt",
              name: "deck.ppt",
              kind: "file",
              extension: "ppt",
              size: 5,
              previewable: false,
            },
          ],
        },
      });
    });
    getVaultFile.mockImplementation(async (_wardId: string, path: string) => {
      if (path === "reports/proposal.docx") {
        return {
          success: true,
          data: {
            kind: "office",
            ward_id: "stock-analysis",
            path,
            extension: "docx",
            contentType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            data: new ArrayBuffer(8),
          },
        };
      }
      if (path === "reports/huge.docx") {
        return {
          success: true,
          data: {
            kind: "office",
            ward_id: "stock-analysis",
            path,
            extension: "docx",
            contentType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            data: new ArrayBuffer(8),
          },
        };
      }
      if (path === "reports/large.txt") {
        return { success: false, error: "HTTP 413: Payload Too Large" };
      }
      return {
        success: true,
        data: {
          kind: "text",
          ward_id: "stock-analysis",
          path,
          name: path.split("/").pop() ?? path,
          extension: path.split(".").pop() ?? "txt",
          size: 11,
          content: textContentFor(path),
        },
      };
    });
    searchVaultFiles.mockResolvedValue({
      success: true,
      data: {
        ward_id: "stock-analysis",
        query: "valuation",
        truncated: false,
        matches: [
          {
            ward_id: "stock-analysis",
            path: "reports/valuation.md",
            name: "valuation.md",
            kind: "file",
            extension: "md",
            size: 11,
            previewable: true,
          },
        ],
      },
    });
    openWard.mockResolvedValue({ success: true, data: { path: "/tmp/stock-analysis" } });
  });

  async function openWardsFromRoot(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByRole("button", { name: "Wards" }));
    await screen.findByRole("button", { name: /stock-analysis/i });
  }

  function expectBreadcrumb(...items: string[]) {
    const breadcrumb = screen.getByLabelText("Breadcrumb");
    for (const item of items) {
      expect(within(breadcrumb).getByText(item)).toBeInTheDocument();
    }
  }

  it("renders the Vault root when opened from the top-level Vault link", async () => {
    render(<VaultPage />);

    await screen.findByRole("button", { name: "Wards" });
    expectBreadcrumb("Vault");
    expect(screen.queryByText("stock-analysis")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Vault explorer")).toBeInTheDocument();
    expect(screen.getByLabelText("Vault file preview")).toBeInTheDocument();
  });

  it("opens the Wards section from the Vault root", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);

    expectBreadcrumb("Vault", "Wards");
    expect(await screen.findByRole("button", { name: /stock-analysis/i })).toBeInTheDocument();
    expect(window.location.search).toBe("?section=wards");
  });

  it("selects a ward, expands a directory, and renders markdown preview", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("stock-analysis", ""));
    expectBreadcrumb("Vault", "Wards", "stock-analysis");
    expect(screen.getAllByText("Ward content").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole("searchbox", { name: "Fuzzy search files in stock-analysis" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "research-lab" })).not.toBeInTheDocument();
    expect(screen.getAllByText("stock-analysis").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("Directory truncated at 1,000 entries.")).toBeInTheDocument();
    expect(window.location.search).toBe("?ward=stock-analysis");

    await user.click(await screen.findByRole("button", { name: /reports/i }));
    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("stock-analysis", "reports"));

    await user.click(await screen.findByRole("button", { name: /valuation\.md/i }));
    await waitFor(() => expect(getVaultFile).toHaveBeenCalledWith("stock-analysis", "reports/valuation.md"));
    const heading = await screen.findByRole("heading", { name: "Valuation" });
    expect(heading).toBeInTheDocument();
    expect(heading.closest(".artifact-slideout__md")).toBeTruthy();
    expect(heading.closest(".vault-markdown")).toBeTruthy();
  });

  it("returns from a selected ward to the Vault wards list from the breadcrumb", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("stock-analysis", ""));
    expect(await screen.findByRole("button", { name: /deck\.ppt/i })).toBeInTheDocument();

    const breadcrumb = screen.getByLabelText("Breadcrumb");
    await user.click(within(breadcrumb).getByRole("button", { name: "Wards" }));

    expect(screen.queryByRole("button", { name: /deck\.ppt/i })).not.toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /stock-analysis/i })).toBeInTheDocument();
    expect(window.location.search).toBe("?section=wards");
  });

  it("returns from a selected ward to the Vault root from the breadcrumb", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("stock-analysis", ""));

    const breadcrumb = screen.getByLabelText("Breadcrumb");
    await user.click(within(breadcrumb).getByRole("button", { name: "Vault" }));

    expect(screen.queryByRole("button", { name: /stock-analysis/i })).not.toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Wards" })).toBeInTheDocument();
    expect(window.location.search).toBe("");
  });

  it("opens the requested ward from the Vault URL", async () => {
    window.history.pushState({}, "", "/vault?ward=stock-analysis");

    render(<VaultPage />);

    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("stock-analysis", ""));
    expect(await screen.findByRole("button", { name: /deck\.ppt/i })).toBeInTheDocument();
    expect(screen.getAllByText("stock-analysis").length).toBeGreaterThanOrEqual(1);
  });

  it("collapses and expands the Vault explorer sidebar", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    expect(await screen.findByLabelText("Vault explorer")).toBeInTheDocument();
    await user.click(screen.getAllByRole("button", { name: "Collapse vault explorer" })[0]);

    expect(screen.queryByLabelText("Vault explorer")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Expand vault explorer" }));

    expect(await screen.findByLabelText("Vault explorer")).toBeInTheDocument();
  });

  it("resizes the Vault explorer pane with the splitter keyboard controls", async () => {
    render(<VaultPage />);

    const splitter = await screen.findByRole("separator", { name: "Resize vault explorer" });
    const split = splitter.parentElement as HTMLElement;
    expect(split.style.getPropertyValue("--vault-explorer-width")).toBe("340px");

    fireEvent.keyDown(splitter, { key: "ArrowRight" });
    expect(splitter).toHaveAttribute("aria-valuenow", "364");
    expect(split.style.getPropertyValue("--vault-explorer-width")).toBe("364px");

    fireEvent.keyDown(splitter, { key: "Home" });
    expect(splitter).toHaveAttribute("aria-valuenow", "240");
    expect(split.style.getPropertyValue("--vault-explorer-width")).toBe("240px");
  });

  it("fuzzy-searches ward files from the sidebar and opens a result", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    expect(screen.queryByRole("searchbox", { name: /fuzzy search files in/i })).not.toBeInTheDocument();

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("stock-analysis", ""));

    await user.type(screen.getByRole("searchbox", { name: "Fuzzy search files in stock-analysis" }), "valuation");
    await waitFor(() => expect(searchVaultFiles).toHaveBeenCalledWith("stock-analysis", "valuation", 30));

    const searchbox = screen.getByRole("searchbox", { name: "Fuzzy search files in stock-analysis" });
    await user.clear(searchbox);
    await user.type(searchbox, "notes");
    await waitFor(() => expect(screen.queryByRole("button", { name: /valuation\.md/i })).not.toBeInTheDocument());

    await user.clear(searchbox);
    await user.type(searchbox, "valuation");
    await user.click(await screen.findByRole("button", { name: /valuation\.md/i }));
    await waitFor(() => expect(getVaultFile).toHaveBeenCalledWith("stock-analysis", "reports/valuation.md"));
    expect(await screen.findByRole("heading", { name: "Valuation" })).toBeInTheDocument();
  });

  it("shows legacy Office files as non-previewable and opens the ward folder", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await user.click(await screen.findByRole("button", { name: /deck\.ppt/i }));

    expect(screen.getByText("Preview not available for .ppt files.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /open ward folder/i }));
    expect(openWard).toHaveBeenCalledWith("stock-analysis");
    expect(getVaultFile).not.toHaveBeenCalledWith("stock-analysis", "deck.ppt");
  });

  it("renders text and code as source previews and HTML in preview mode", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await user.click(await screen.findByRole("button", { name: /reports/i }));

    await user.click(await screen.findByRole("button", { name: /notes\.txt/i }));
    expect(await screen.findByText("plain notes")).toBeInTheDocument();

    await user.click(await screen.findByRole("button", { name: /app\.py/i }));
    expect(await screen.findByText("print('vault')")).toBeInTheDocument();

    await user.click(await screen.findByRole("button", { name: /index\.html/i }));
    const preview = await screen.findByTitle("HTML preview: index.html");
    expect(preview).toBeInstanceOf(HTMLIFrameElement);
    expect(preview).toHaveAttribute("sandbox", "");
    expect(preview).toHaveAttribute("srcdoc", "<script>alert('x')</script>");
    expect(screen.queryByText("<script>alert('x')</script>")).not.toBeInTheDocument();
  });

  it("keeps Office files out of the browser previewer", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await user.click(await screen.findByRole("button", { name: /reports/i }));
    await user.click(await screen.findByRole("button", { name: /proposal\.docx/i }));

    expect(await screen.findByText(/Office previews are disabled for safety/)).toBeInTheDocument();
  });

  it("renders oversized and Office safety messages", async () => {
    const user = userEvent.setup();
    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await user.click(await screen.findByRole("button", { name: /reports/i }));

    await user.click(await screen.findByRole("button", { name: /large\.txt/i }));
    expect(await screen.findByText("HTTP 413: Payload Too Large")).toBeInTheDocument();

    await user.click(await screen.findByRole("button", { name: /huge\.docx/i }));
    expect(await screen.findByText(/Office previews are disabled for safety/)).toBeInTheDocument();
  });

  it("ignores stale ward tree responses after switching wards", async () => {
    const user = userEvent.setup();
    const wardAResponse = deferred<unknown>();
    listVaultWards.mockResolvedValue({
      success: true,
      data: {
        wards: [
          { id: "ward-a", name: "ward-a" },
          { id: "ward-b", name: "ward-b" },
        ],
      },
    });
    getVaultTree.mockImplementation((wardId: string, path = "") => {
      if (wardId === "ward-a" && path === "") return wardAResponse.promise;
      return Promise.resolve({
        success: true,
        data: {
          ward_id: wardId,
          path,
          truncated: false,
          children: [
            {
              ward_id: wardId,
              path: `${wardId}.md`,
              name: `${wardId}.md`,
              kind: "file",
              extension: "md",
              size: 8,
              previewable: true,
            },
          ],
        },
      });
    });

    render(<VaultPage />);

    await user.click(await screen.findByRole("button", { name: "Wards" }));
    await user.click(await screen.findByRole("button", { name: "ward-a" }));
    await waitFor(() => expect(getVaultTree).toHaveBeenCalledWith("ward-a", ""));
    await user.click(screen.getByRole("button", { name: "Wards" }));
    await user.click(await screen.findByRole("button", { name: "ward-b" }));

    expect(await screen.findByRole("button", { name: "ward-b.md" })).toBeInTheDocument();
    await act(async () => {
      wardAResponse.resolve({
        success: true,
        data: {
          ward_id: "ward-a",
          path: "",
          truncated: false,
          children: [
            {
              ward_id: "ward-a",
              path: "ward-a.md",
              name: "ward-a.md",
              kind: "file",
              extension: "md",
              size: 8,
              previewable: true,
            },
          ],
        },
      });
      await wardAResponse.promise;
    });

    expect(screen.queryByRole("button", { name: "ward-a.md" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "ward-b.md" })).toBeInTheDocument();
  });

  it("ignores stale file preview responses after selecting another file", async () => {
    const user = userEvent.setup();
    const notesResponse = deferred<unknown>();
    getVaultFile.mockImplementation(async (_wardId: string, path: string) => {
      if (path === "reports/notes.txt") return notesResponse.promise;
      return {
        success: true,
        data: {
          kind: "text",
          ward_id: "stock-analysis",
          path,
          name: path.split("/").pop() ?? path,
          extension: path.split(".").pop() ?? "txt",
          size: 11,
          content: textContentFor(path),
        },
      };
    });

    render(<VaultPage />);

    await openWardsFromRoot(user);
    await user.click(await screen.findByRole("button", { name: /stock-analysis/i }));
    await user.click(await screen.findByRole("button", { name: /reports/i }));
    await user.click(await screen.findByRole("button", { name: /notes\.txt/i }));
    await waitFor(() => expect(getVaultFile).toHaveBeenCalledWith("stock-analysis", "reports/notes.txt"));
    await user.click(await screen.findByRole("button", { name: /app\.py/i }));

    expect(await screen.findByText("print('vault')")).toBeInTheDocument();
    await act(async () => {
      notesResponse.resolve({
        success: true,
        data: {
          kind: "text",
          ward_id: "stock-analysis",
          path: "reports/notes.txt",
          name: "notes.txt",
          extension: "txt",
          size: 11,
          content: "stale notes",
        },
      });
      await notesResponse.promise;
    });

    expect(screen.queryByText("stale notes")).not.toBeInTheDocument();
    expect(screen.getByText("print('vault')")).toBeInTheDocument();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

function textContentFor(path: string): string {
  if (path.endsWith("valuation.md")) return "# Valuation";
  if (path.endsWith("notes.txt")) return "plain notes";
  if (path.endsWith("app.py")) return "print('vault')";
  if (path.endsWith("index.html")) return "<script>alert('x')</script>";
  return "";
}
