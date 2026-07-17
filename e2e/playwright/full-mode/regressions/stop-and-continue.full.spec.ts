import { expect } from "@playwright/test";
import { bootFullMode } from "../../lib/harness-full";

const { test, handle } = bootFullMode({ fixture: "stop-and-continue" });

test.describe("regression: stop mid-session, continue, root completes", () => {
  // Locks in:
  //   - PR #67: cancel_session + reactivate clear pending_delegations and
  //     continuation_needed (the bookkeeping reset is also covered by the
  //     unit test lifecycle_tests.rs::stop_execution_resets_pending_delegations).
  //   - The runner decomposition: ContinuationWatcher + DelegationDispatcher
  //     + ExecutionStream + InvokeBootstrap correctly cooperate across a stop
  //     boundary — specifically that a session stopped by the user can accept
  //     a fresh invoke and run to completion.
  //
  // Fixture: stop-and-continue
  //   FIFO LLM response 1: respond("First response before stop.")
  //   FIFO LLM response 2: respond("Done after continuation.")
  //
  // Note: in full-mode, zerod processes tool calls itself; mock-llm serves
  // responses in FIFO order. The stop attempt is best-effort because the
  // first respond completes in milliseconds — whether stop succeeds or not,
  // the continuation turn must still reach completed.

  test("root execution reaches 'completed' after stop+continue", async ({ page }) => {
    // The boot script passes gateway_ws pointing at zerod's --ws-port, but zerod
    // runs in unified mode (HTTP+WS on the same port). Fix the WS URL to use the
    // HTTP port with the /ws path, which is where zerod's WS upgrade actually lives.
    const gatewayHttpBase = handle.gatewayUrl("/").replace(/\/$/, "");
    const correctWsUrl = gatewayHttpBase.replace(/^http:/, "ws:") + "/ws";
    const rawUrl = handle.uiUrl("/research");
    const fixedUrl = rawUrl.includes("gateway_ws=")
      ? rawUrl.replace(/gateway_ws=[^&]*/, `gateway_ws=${encodeURIComponent(correctWsUrl)}`)
      : rawUrl + `&gateway_ws=${encodeURIComponent(correctWsUrl)}`;
    await page.goto(fixedUrl);

    // Turn 1 — kick off the session.
    await page.locator("textarea").fill("Build something that takes a while.");
    await page.locator('button[title="Send message"]').click();

    // Wait for the session URL to flip (zerod created the session).
    await expect.poll(() => page.url(), { timeout: 15_000 })
      .toMatch(/\/research\/sess-/);

    const sessionId = page.url().match(/sess-[a-zA-Z0-9-]+/)?.[0];
    expect(sessionId).toBeTruthy();

    // Consume the fixture's first response before the best-effort stop. If
    // cancel wins the race with mock-llm, the continuation would consume this
    // first FIFO response and the test could not prove persistence of the
    // second continuation response.
    await expect(
      page.locator(".session-turn").first().locator(".research-msg--assistant"),
    ).toContainText("First response before stop.", { timeout: 15_000 });

    // Best-effort stop: attempt via HTTP cancel. This succeeds when the session
    // is still RUNNING; it fails gracefully when already COMPLETED (which is
    // expected since mock-llm responds instantly). Either outcome is fine —
    // the continuation turn must complete regardless.
    const cancelUrl = handle.gatewayUrl(`/api/gateway/cancel/${sessionId}`);
    await fetch(cancelUrl, { method: "POST" }).catch(() => {
      // Swallow network errors (session may already be completed).
    });

    // Wait for the first turn to settle in zerod (either completed or crashed).
    // The stop attempt is best-effort: mock-llm responds in milliseconds, so the
    // session may have already completed before the cancel reached zerod.
    // Either outcome is fine — both are terminal states for the first turn.
    await expect.poll(async () => {
      try {
        const res = await fetch(
          handle.gatewayUrl(`/api/executions/v2/sessions/full?limit=200`)
        );
        if (!res.ok) return null;
        const list: any[] = await res.json();
        const ours = list.find((s: any) => s.id === sessionId);
        return ours?.status ?? null;
      } catch {
        return null;
      }
    }, { timeout: 15_000, intervals: [200, 500] }).toMatch(/^(completed|crashed)$/);

    // Turn 2 — submit the continuation directly via the gateway REST API.
    // The UI's composer stays disabled after agent_stopped (AGENT_STOPPED
    // only updates turn-level status, not state.status), so we bypass it
    // and use POST /api/gateway/submit with the existing session_id.
    // This is the correct continuation path: zerod picks up the session,
    // reactivates it, clears bookkeeping (pending_delegations, continuation_needed),
    // and runs a fresh root turn to completion.
    const submitUrl = handle.gatewayUrl(`/api/gateway/submit`);
    const submitRes = await fetch(submitUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        agent_id: "root",
        message: "continue",
        session_id: sessionId,
        source: "api",
      }),
    });
    expect(submitRes.ok, `submit failed: ${submitRes.status}`).toBeTruthy();

    // Poll zerod's state API until the session reaches completed.
    // The contract: root must COMPLETE, not stay stuck at running/crashed.
    await expect.poll(async () => {
      try {
        const res = await fetch(
          handle.gatewayUrl(`/api/executions/v2/sessions/full?limit=200`)
        );
        if (!res.ok) return null;
        const list: any[] = await res.json();
        const ours = list.find((s: any) => s.id === sessionId);
        return ours?.status ?? null;
      } catch {
        return null;
      }
    }, { timeout: 30_000, intervals: [500, 1000, 2000] }).toBe("completed");

    // Full bookkeeping assertion — the runner decomposition contract.
    const stateRes = await fetch(
      handle.gatewayUrl(`/api/executions/v2/sessions/full?limit=200`)
    );
    expect(stateRes.ok).toBeTruthy();
    const list: any[] = await stateRes.json();
    const ours = list.find((s: any) => s.id === sessionId);

    expect(ours, `session ${sessionId} not found in /v2/sessions/full`).toBeTruthy();
    expect(ours.status).toBe("completed");
    expect(ours.pending_delegations).toBe(0);
    expect(ours.continuation_needed).toBe(false);

    // The root execution itself must be completed with a non-null ended_at.
    const root = ours.executions?.find((e: any) => e.delegation_type === "root");
    expect(root, "root execution not found").toBeTruthy();
    expect(root.status).toBe("completed");
    expect(root.ended_at).not.toBeNull();

    // Reload from durable storage rather than relying on the live WS
    // TurnComplete event. The continuation runner must have persisted its
    // `respond()` argument, and snapshot recovery must prefer this later
    // terminal answer over the earlier progress response. A single message
    // also guards against duplicate terminal delivery rendering twice.
    // The API continuation does not own navigation, so explicitly open the
    // durable session URL. This is equivalent to a browser refresh while
    // avoiding any incidental landing-page navigation from the stop flow.
    await page.goto(handle.uiUrl(`/research/${sessionId}`));
    const finalResponses = page
      .locator(".research-msg--assistant")
      .filter({ hasText: "Done after continuation." });
    await expect(finalResponses).toHaveCount(1, { timeout: 15_000 });
    await expect(finalResponses.first()).toContainText("Done after continuation.");
  });
});
