# Persistent work surfaces — manual QA

Status: completed 2026-07-28

- [x] Enable “Persist infographics” in Settings and observe the saved state
  without a restart.
- [x] Restore one saved infographic in Quick Chat, reload, and verify it
  renders exactly once.
- [x] Open a completed Research session, reload, and verify its saved surface
  renders exactly once.
- [x] Disable persistence and verify the API returns no saved surfaces while
  retaining existing rows; re-enable and verify those rows are restorable.
- [x] Confirm “Clear saved infographics,” verify the fixed confirmation body
  and deleted count, and verify no live-surface reset is dispatched.
- [x] Verify cleared rows no longer restore and the owning session remains.

Result: passed. The visible Settings, Quick Chat, and Research gestures ran in
Chromium through `tests/e2e/persistent-surfaces.spec.ts` (2 tests). The
disable/re-enable and clear storage invariants ran through
`gateway/tests/saved_surfaces.rs` plus execution-state repository tests.
