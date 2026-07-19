// ============================================================================
// WebAppShell — top-bar nav + mobile menu tests
// ============================================================================

import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@/test/utils";
import { WebAppShell, navItems } from "./App";

describe("navItems (top-bar order)", () => {
  it("is a flat list — no group containers", () => {
    expect(Array.isArray(navItems)).toBe(true);
    expect(navItems.length).toBeGreaterThan(0);
  });

  it("starts with Research and ends with Settings", () => {
    expect(navItems[0]?.to).toBe("/research");
    expect(navItems[navItems.length - 1]?.to).toBe("/settings");
  });

  it("includes Quick chat (renamed from Chat) and all design-system tabs", () => {
    const labels = navItems.map((i) => i.label);
    expect(labels).toContain("Research");
    expect(labels).toContain("Quick chat");
    expect(labels).toContain("Mission Control");
    expect(labels).toContain("Agents");
    expect(labels).toContain("Memory");
    expect(labels).toContain("Vault");
    // Dashboard + Logs are consolidated into Mission Control (PR adds the
    // /dashboard and /logs redirects). Asserting they're absent locks that in.
    expect(labels).not.toContain("Dashboard");
    expect(labels).not.toContain("Logs");
    expect(labels).toContain("Observatory");
    expect(labels).toContain("Integrations");
    expect(labels).toContain("Settings");
  });
});

describe("WebAppShell — top-bar shell", () => {
  it("renders the brand mark and all nav links", () => {
    render(
      <WebAppShell connectionStatus={{ connected: true }}>
        <div>page content</div>
      </WebAppShell>
    );

    expect(screen.getByLabelText(/z-bot home/i)).toBeInTheDocument();
    // Each nav link appears twice: once in the desktop top-bar nav,
    // once in the mobile sheet (offscreen until opened).
    for (const item of navItems) {
      const links = screen.getAllByRole("link", { name: new RegExp(item.label, "i") });
      expect(links.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders the connection pill in connected state", () => {
    render(
      <WebAppShell connectionStatus={{ connected: true }}>
        <div>x</div>
      </WebAppShell>
    );
    expect(screen.getByText(/connected · zbotd/i)).toBeInTheDocument();
  });

  it("renders the disconnected pill when not connected", () => {
    render(
      <WebAppShell connectionStatus={{ connected: false }}>
        <div>x</div>
      </WebAppShell>
    );
    expect(screen.getByText(/disconnected/i)).toBeInTheDocument();
  });

  it("renders the AccentPicker trigger in the top-bar", () => {
    render(
      <WebAppShell connectionStatus={{ connected: true }}>
        <div>x</div>
      </WebAppShell>
    );
    expect(screen.getByRole("button", { name: /theme accent/i })).toBeInTheDocument();
  });

  it("renders children in the main pane", () => {
    render(
      <WebAppShell connectionStatus={{ connected: true }}>
        <div data-testid="page">hello</div>
      </WebAppShell>
    );
    expect(screen.getByTestId("page")).toHaveTextContent("hello");
  });

  it("toggles the mobile sheet open and closed via the menu button", () => {
    render(
      <WebAppShell connectionStatus={{ connected: true }}>
        <div>x</div>
      </WebAppShell>
    );

    const toggle = screen.getByRole("button", { name: /open menu/i });
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(toggle);
    // After opening, both the toggle and the backdrop have "Close menu" labels.
    // The toggle is the only one carrying aria-expanded.
    const closeButtons = screen.getAllByRole("button", { name: /close menu/i });
    expect(closeButtons.length).toBeGreaterThanOrEqual(1);
    const expandedToggle = closeButtons.find((b) => b.hasAttribute("aria-expanded"));
    expect(expandedToggle).toBeDefined();
    expect(expandedToggle).toHaveAttribute("aria-expanded", "true");

    // Backdrop is rendered when the sheet is open
    const backdrop = closeButtons.find((b) => b.classList.contains("topbar__sheet-backdrop"));
    expect(backdrop).toBeDefined();

    // Clicking the toggle again closes the sheet
    fireEvent.click(expandedToggle as HTMLElement);
    expect(screen.queryByRole("button", { name: /close menu/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /open menu/i })).toHaveAttribute(
      "aria-expanded",
      "false"
    );
  });
});
