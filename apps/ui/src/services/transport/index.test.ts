// ============================================================================
// transport/index — defaultConfig + URL-param override
// Asserts the same-origin behaviour that lets mobile clients hit the daemon
// on its LAN address without a port mismatch.
// ============================================================================

import { describe, it, expect, beforeEach, vi } from "vitest";

// We have to re-import the module per test because configureGateway runs
// once at module load. vi.resetModules() between tests gives us a fresh
// closure each time.

beforeEach(() => {
  vi.resetModules();
  // jsdom provides a window — wipe any stale __ZERO_CONFIG__ between tests.
  delete (window as { __ZERO_CONFIG__?: unknown }).__ZERO_CONFIG__;
});

async function importTransport() {
  return await import("./index");
}

describe("transport defaultConfig (browser, no overrides)", () => {
  it("uses an empty httpUrl so /api fetches resolve same-origin", async () => {
    // Force jsdom to a known origin — production-like phone scenario.
    history.replaceState(null, "", "/research");
    const { initializeTransport, getTransport } = await importTransport();
    await initializeTransport();
    const t = await getTransport();
    const cfg = (t as unknown as { config: { httpUrl: string; wsUrl: string } }).config;
    expect(cfg.httpUrl).toBe("");
  });

  it("builds wsUrl from window.location.host so the WS port matches the page port", async () => {
    history.replaceState(null, "", "/research");
    const { initializeTransport, getTransport } = await importTransport();
    await initializeTransport();
    const t = await getTransport();
    const cfg = (t as unknown as { config: { httpUrl: string; wsUrl: string } }).config;
    // jsdom's default origin is `http://localhost:3000` (configured in vitest).
    // The exact host varies by setup; what matters is the WS reuses it.
    expect(cfg.wsUrl).toMatch(/^ws:\/\/[^/]+\/ws$/);
    expect(cfg.wsUrl).toContain(window.location.host);
  });

  it("does not bake the gateway port into the WS URL", async () => {
    history.replaceState(null, "", "/research");
    const { initializeTransport, getTransport } = await importTransport();
    await initializeTransport();
    const t = await getTransport();
    const cfg = (t as unknown as { config: { wsUrl: string } }).config;
    // The hardcoded gateway port should NOT appear unless the page is
    // actually being served on that port (which jsdom isn't).
    if (!window.location.host.includes("18791")) {
      expect(cfg.wsUrl).not.toContain(":18791");
    }
  });
});

describe("transport defaultConfig (URL param overrides)", () => {
  it("respects ?gateway_http=... when set", async () => {
    history.replaceState(null, "", "/research?gateway_http=http%3A%2F%2Fcustom%3A9999");
    const { initializeTransport, getTransport } = await importTransport();
    await initializeTransport();
    const t = await getTransport();
    const cfg = (t as unknown as { config: { httpUrl: string } }).config;
    expect(cfg.httpUrl).toBe("http://custom:9999");
  });

  it("respects ?gateway_ws=... when set", async () => {
    history.replaceState(null, "", "/research?gateway_ws=ws%3A%2F%2Fcustom%3A9998%2Fws");
    const { initializeTransport, getTransport } = await importTransport();
    await initializeTransport();
    const t = await getTransport();
    const cfg = (t as unknown as { config: { wsUrl: string } }).config;
    expect(cfg.wsUrl).toBe("ws://custom:9998/ws");
  });

  it("falls back to same-origin for the side that wasn't overridden", async () => {
    // Override only the WS URL — HTTP should stay same-origin (empty).
    history.replaceState(null, "", "/research?gateway_ws=ws%3A%2F%2Fcustom%3A9998%2Fws");
    const { initializeTransport, getTransport } = await importTransport();
    await initializeTransport();
    const t = await getTransport();
    const cfg = (t as unknown as { config: { httpUrl: string } }).config;
    expect(cfg.httpUrl).toBe("");
  });
});
