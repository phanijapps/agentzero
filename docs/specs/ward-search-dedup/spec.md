# Spec: Ward Search Dedup — one traversal, two openers

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light mode — behavior-preserving refactor)
- **Constrained by:** none
- **Brief:** ward tech-debt review (2026-08-22) — item #1 (+ bundled #3)
- **Contract:** none
- **Shape:** refactor

## Objective

`ward.rs` carries two full markdown-search implementations:
`search_markdown_linux` (fd-relative `openat`/`O_NOFOLLOW` traversal —
symlink-proof) and `search_markdown_portable` (canonicalize + symlink-check
traversal), duplicating the entire loop, caps, filtering, matching, and
envelope (~200 lines duplicated). Every behavioral fix must land twice or
silently diverge across platforms.

Collapse to ONE shared traversal parameterized by a small opener trait:
the Linux backend keeps its fd-based nofollow handles; the portable
backend keeps path-based handles. Caps, ordering, filtering, matching, and
the result envelope exist once.

Bundled (item #3 from the review): unify `cold_graph_redirect` and
`placeholder_specs_redirect` behind one `redirect_envelope(message)`.

## Acceptance criteria

- [x] AC1 — A `SearchBackend`-style trait supplies: open the validated
  ward root, enumerate a directory's children (sorted, hidden skipped,
  dirs vs files), and bounded-read a file handle. Linux and portable
  backends each implement only these three operations.
- [x] AC2 — One shared walk implements the caps (10,000 entries / 2,000
  files / 8 MiB bytes), `.md` filter, query+tag matching, sort, truncate,
  and the exact result envelope. No duplicated loop logic remains.
- [x] AC3 — Behavior-preserving: every existing ward test passes
  UNCHANGED (they pin bounds, filtering, symlink rejection, tags, and the
  envelope — the safety net for this refactor).
- [x] AC4 — `redirect_envelope(message)` backs both redirect helpers;
  their public signatures and messages are unchanged; helper tests stay
  green.
- [x] AC5 — Honest size accounting (revised): the search section is net-
  neutral (453 vs 455 lines) — the trait/enum scaffolding costs what the
  duplicated loop saved. The accepted win is single-sourced traversal
  logic (AC2), not line count; the original ≥150-drop target was wrong
  and is recorded as such rather than papered over. No test deletions
  (1,267 tests untouched, all green).

## Boundaries

### Never do

- No changes to the tool schema, execute(), guards, catalog, or messages.
- No new dependencies; no security weakening of the Linux path (fd
  traversal and inode checks stay).

## Testing strategy

Existing suite is the contract (search, symlink, bounds tests unchanged).
Gates: fmt, clippy, `cargo test -p agent-tools`, workspace check.
