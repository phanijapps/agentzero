# [IMPLEMENTED 2026-05-03] Setup Wizard Personality Presets — Design

**Status:** Design (awaiting review)
**Date:** 2026-05-02
**Owner:** phanijapps
**Target branch:** `develop`

## Problem

The first-run setup wizard offers four personality presets for the root agent: `brahmi`, `johnnylever`, `zbot`, `custom`. Two of these names ("Brahmi", "JohnnyLever") are placeholder personas the user wants replaced with the identifications they actually use:

- "Bhrami Software Engineer"
- "Gajala CEO Sonic Solutions"

The taglines and the other two presets (`zbot`, `custom`) stay.

## Goals

1. Replace the first two preset entries with the new identifications.
2. Update the wizard's initial state so a fresh install defaults to "Bhrami Software Engineer" instead of "Brahmi".
3. Preserve hydration behavior so existing users with a stored name like "Brahmi" still load cleanly (falling back to the `custom` preset).

## Non-goals

- Wizard memory-ingestion changes — the existing `aboutMe` → `user.profile` pinned memory fact (per `ReviewStep.tsx:147-155`) is sufficient.
- Restructuring of `SOUL.md` generation — the gateway already substitutes `agentName` into the template; the new longer name fits the same way.
- Adding new preset slots, new fields, or richer personality knobs.
- Migration of existing installs — the wizard's hydration code already handles unknown stored names by selecting `custom`.

## Changes

### File 1: `apps/ui/src/features/setup/presets.ts`

Replace the entire `NAME_PRESETS` array. Two entries change in place; two are unchanged:

```typescript
export const NAME_PRESETS: NamePreset[] = [
  { id: "bhrami", name: "Bhrami Software Engineer",   emoji: "🎭", tagline: "Witty, resourceful, always has a plan" },
  { id: "gajala", name: "Gajala CEO Sonic Solutions", emoji: "😂", tagline: "Energetic, creative, makes work fun" },
  { id: "zbot",   name: "z-Bot",                      emoji: "🤖", tagline: "Professional, focused, gets things done" },
  { id: "custom", name: "Custom...",                  emoji: "✨", tagline: "Choose your own name" },
];
```

Why these specific edits:

- **`brahmi` → `bhrami`** — name "Brahmi" → "Bhrami Software Engineer"; ID renamed for internal consistency. Tagline unchanged.
- **`johnnylever` → `gajala`** — name "JohnnyLever" → "Gajala CEO Sonic Solutions"; ID renamed for internal consistency. Tagline unchanged.
- **`zbot` and `custom`** — untouched.

ID rename rationale: these IDs are internal-only (used as `state.namePreset`). Renaming them to match the new names is cleaner. Existing installs that stored the old IDs in component-local state lose that local state when the user re-runs the wizard, but the user's actual data (settings.json `agentName`) is preserved. The hydration logic at `SetupWizard.tsx:117` and `:150` falls back to `namePreset = "custom"` for any unmatched name, so there's no broken state — just a cosmetic re-selection on first re-run.

### File 2: `apps/ui/src/features/setup/SetupWizard.tsx`

Two lines change in the wizard's initial reducer state:

```diff
-  agentName: "Brahmi",
-  namePreset: "brahmi",
+  agentName: "Bhrami Software Engineer",
+  namePreset: "bhrami",
```

Match the new default preset.

## Behavior preserved (no code change required)

- **Memory ingestion.** `ReviewStep.tsx:147-155` writes the `aboutMe` textarea content as a pinned memory fact (`category=user`, `key=user.profile`, `confidence=0.95`). Unchanged.
- **SOUL.md generation.** The gateway substitutes `agentName` into the SOUL template via the existing flow (settings.json → root agent's `displayName` → SOUL.md regeneration). The longer name "Bhrami Software Engineer" fits the same way "Brahmi" did.
- **Hydration on re-run.** `SetupWizard.tsx:117` matches the stored `agentName` against `NAME_PRESETS`; if no match, it falls back to `namePreset = "custom"`. So an install that stored "Brahmi" will hydrate as `agentName="Brahmi", namePreset="custom"` after this change — the user re-running the wizard sees their existing name preserved with the `Custom...` preset selected, and can pick one of the new presets if they want.

## Testing

Manual only — no automated test changes.

- [ ] Open the wizard at `/setup` on a fresh install (no settings.json). Verify the four presets show: "Bhrami Software Engineer", "Gajala CEO Sonic Solutions", "z-Bot", "Custom...". Click each in turn; the `agentName` field updates correctly.
- [ ] Default state on first wizard load: "Bhrami Software Engineer" is pre-selected.
- [ ] Click `Custom...`; the input becomes editable; type any string; submit; verify `agentName` is preserved.
- [ ] Existing install (settings.json with `agentName: "Brahmi"`): wizard hydrates to `agentName="Brahmi", namePreset="custom"`. No errors, no data loss.
- [ ] Build: `cd apps/ui && npm run build` clean.
- [ ] Lint: `cd apps/ui && npm run lint` clean for the touched files.

## Out of scope

- Customization tab for editing markdowns in `<vault>/config/` — separate spec / brainstorm queued (Task #53).
- Multi-fact distillation of the `aboutMe` textarea — out of scope; user explicitly said leave the existing memory ingestion alone.

## References

- `apps/ui/src/features/setup/presets.ts` — preset definitions.
- `apps/ui/src/features/setup/SetupWizard.tsx:69-70` — initial reducer state.
- `apps/ui/src/features/setup/SetupWizard.tsx:117,150` — hydration logic.
- `apps/ui/src/features/setup/steps/ReviewStep.tsx:147-155` — existing `user.profile` memory ingestion (preserved).
- Memory: `feedback_wizard_deltas.md` (re-running wizard must apply deltas only) — preserved by this change.
- Memory: `feedback_soft_reference_name.md` (root agent ID stays "root", name is `displayName` only) — preserved by this change.
