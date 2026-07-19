# Ward Curator — Spec (Phases A + B + C)

**Date:** 2026-05-23
**Goal:** Close gap #1 from `2026-05-22-hermes-comparison-gaps.md` by giving z-Bot an autonomous **ward** self-improvement loop — the substrate equivalent of Hermes's Curator. Hermes's "skill" is structurally a z-Bot ward; this spec applies the Curator pattern to the correct substrate.
**Supersedes:** `2026-05-22-zbot-curator-design.md` (skill-framed first draft).

---

## 0. Vocabulary & provenance

- **Bundled ward:** seeded by code at boot (`scratch`, `wiki` from `ensure_wards_dir`/`ensure_wiki_ward`). **Never** touched by the curator.
- **User-authored ward:** created by the user manually (file copy / external tool). **Never** touched by the curator.
- **Agent-authored ward:** created by the cold-path planner → builder → solution-agent flow. **Only these are eligible for curator action.**

Provenance is recorded once at creation in `wards/.usage.json` as `created_by` (`bundled` | `user` | `agent`). A ward dir with no sidecar entry defaults to `user` — a conservative unknown.

---

## 1. Phase A — Per-ward usage telemetry

### 1.1 Sidecar schema

Single file: `~/Documents/zbot/wards/.usage.json`.

```json
{
  "automotive-research": {
    "use_count": 14,
    "patch_count": 2,
    "last_used_at": "2026-05-22T16:47:33Z",
    "last_patched_at": "2026-05-12T08:00:00Z",
    "created_at": "2026-05-10T09:00:00Z",
    "created_by": "agent",
    "state": "active",
    "pinned": false,
    "archived_at": null
  },
  "wiki": {
    "use_count": 8,
    "last_used_at": "2026-05-22T16:49:00Z",
    "created_by": "bundled",
    "state": "active",
    "pinned": true,
    "patch_count": 0,
    "created_at": "2026-04-15T...",
    "last_patched_at": null,
    "archived_at": null
  }
}
```

Atomic writes only: write to `.usage.json.tmp`, then `rename(2)`. Acquire an `fs2::FileExt::lock_exclusive` around every read-modify-write to prevent corruption under concurrent bumps. Telemetry writes are best-effort — if the lock can't be acquired in 250 ms or the write fails, log a warning and continue. **Telemetry must never block delegation.**

### 1.2 Bump points

| Event | Field updated | Where |
| --- | --- | --- |
| Ward delegation begins (`delegate_to_agent(agent_id="ward:<name>")`) | `use_count += 1`, `last_used_at = now` | `spawn_delegated_agent` in `delegation/spawn.rs`, right after `child_agent_id.strip_prefix("ward:")` resolves the name (same place P5 hooks in) |
| Any agent tool writes to a file under `wards/<name>/AGENTS.md` or `wards/<name>/memory-bank/` | `patch_count += 1`, `last_patched_at = now` | inside `write_file` / `edit_file` post-write; small helper `bump_patch_if_ward_file(path)` |
| Ward scaffolded by the cold path | `created_by = "agent"`, `created_at = now`, `use_count = 0`, `state = "active"` | `gateway-execution/src/invoke/ward_scaffolding.rs` after the directory is materialised |
| Ward seeded at boot (`ensure_wards_dir` / `ensure_wiki_ward`) | `created_by = "bundled"` | `state/mod.rs` after `std::fs::write(agents_md, ...)` |
| Ward dir found on disk with no sidecar entry | lazy insert with `created_by = "user"`, `created_at = mtime(AGENTS.md)` | first read in any Phase B/C call |

### 1.3 New module

`gateway/gateway-services/src/ward_usage.rs` — `WardUsage` service with `bump_use`, `bump_patch`, `mark_created`, `set_state`, `set_pinned`, `archive`, `load`, `save_atomic`. Used from `spawn.rs`, `ward_scaffolding.rs`, `state/mod.rs`, and Phases B + C.

### 1.4 Read API

| Endpoint | Returns |
| --- | --- |
| `GET /api/wards/usage` | Full sidecar (UI / debugging) |
| `GET /api/wards/usage/<ward>` | Single ward record (`404` if missing) |
| `POST /api/wards/usage/<ward>/pin` | `{"pinned": true \| false}` → toggles `pinned` |

---

## 2. Phase B — Heuristic curator + cleanup endpoint

### 2.1 Algorithm

For each ward with `created_by == "agent"` and `pinned == false`:

```
anchor = max(last_used_at, last_patched_at, created_at)
age_days = (now - anchor).whole_days()
```

Apply the **first** matching rule:

| Condition | New state | Action |
| --- | --- | --- |
| `age_days > archive_days` | `archived` | `mv wards/<name>/ wards/_archive/<name>/`, set `archived_at = now`, write the sidecar |
| `age_days > stale_days` AND current `state == "active"` | `stale` | sidecar only |
| `age_days <= stale_days` AND current `state == "stale"` | `active` | sidecar only (reactivate) |

Defaults: `stale_days = 30`, `archive_days = 90`.

Pinned, bundled, and user-authored wards are counted in `scanned` but listed under `skipped_*`.

### 2.2 Cleanup endpoint

```http
POST /api/wards/curator/cleanup
Content-Type: application/json

{
  "dry_run": false,
  "stale_days": 30,
  "archive_days": 90
}
```

All body fields optional; defaults come from `settings.json` (§4).

**Response (200):**

```json
{
  "ok": true,
  "ran_at": "2026-05-23T03:00:00Z",
  "dry_run": false,
  "scanned": 9,
  "skipped_pinned": 1,
  "skipped_non_agent": 4,
  "transitions": [
    {
      "ward": "scratch-vehicle-history",
      "from": "active",
      "to": "stale",
      "anchor": "2026-04-20T10:00:00Z",
      "age_days": 33,
      "reason": "no activity in 33d"
    },
    {
      "ward": "one-off-q1-report",
      "from": "stale",
      "to": "archived",
      "anchor": "2026-02-15T08:00:00Z",
      "age_days": 97,
      "archive_path": "wards/_archive/one-off-q1-report/",
      "reason": "no activity in 97d"
    }
  ],
  "backup_path": "wards/_curator_backups/2026-05-23T030000Z.tar.gz",
  "report_path": "data/curator_logs/2026-05-23T030000Z/REPORT.md"
}
```

**Status codes:**

| Code | Meaning |
| --- | --- |
| `200` | Ran successfully (zero transitions is OK) |
| `409` | Another curator pass in progress — advisory file lock on `_curator_backups/.lock` |
| `500` | Mid-run failure; partial response includes the backup path so the user can restore |

The endpoint is fully self-contained — no sleep-worker integration. **Dry-run** skips backup creation and never mutates state.

### 2.3 Backup

Pre-run, the endpoint writes `wards/_curator_backups/<utc-iso>.tar.gz` containing the whole `wards/` tree **except** `_curator_backups/` and `_archive/` (those are recoverable from prior backups), plus `.usage.json`. Implementation: `tar` + `flate2::write::GzEncoder` from Rust — no shell-out.

Retention: keep the most recent `backup_keep` (default 5). Older are deleted *after* the new one writes successfully.

Dry-run skips backup.

### 2.4 Audit log

`data/curator_logs/<ts>/`:
- `run.json` — machine-readable: pre/post sidecar snapshots, transition list, backup path, errors.
- `REPORT.md` — human: one table of transitions, one of skipped wards with reasons, head + tail of the sidecar diff.

Audit logs are never auto-deleted.

### 2.5 Restore endpoint

```http
POST /api/wards/curator/restore
Content-Type: application/json

{ "backup": "2026-05-23T030000Z" }
```

Untars the named snapshot back over `wards/`, restoring `.usage.json` too. Returns the list of restored files and any conflicts.

### 2.6 Default cron entry (weekly schedule)

Add to `gateway/templates/default_cron.json`:

```json
{
  "id": "ward-curator-cleanup-weekly",
  "name": "Ward curator weekly cleanup",
  "schedule": "0 0 3 * * 1",
  "agent_id": "general-purpose",
  "message": "Run `curl -sS -X POST http://localhost:${ZBOT_API_PORT:-8080}/api/wards/curator/cleanup -H 'Content-Type: application/json' -d '{}'` and report the transitions, scanned count, and backup_path from the JSON response. If the response indicates errors, surface them verbatim.",
  "respond_to": [],
  "enabled": true,
  "metadata": {
    "source": "default_cron.json",
    "purpose": "ward-curator-layer-1"
  }
}
```

Schedule is **6-field** (`sec min hour day month weekday`) per the project's cron convention — `0 0 3 * * 1` = Mondays at 03:00 local. An LLM-less HTTP cron-action type is a tempting follow-up (no agent in the loop), but is **out of scope** here.

---

## 3. Phase C — LLM consolidation curator

### 3.1 Endpoint

```http
POST /api/wards/curator/consolidate
Content-Type: application/json

{
  "dry_run": true,
  "max_consolidations": 5
}
```

`dry_run` defaults to **true** — consolidation is heavier and rarer than cleanup, so mutation is explicit. The endpoint internally invokes Phase B's cleanup logic first (with `dry_run = true`) so the curator-agent sees a fresh view and doesn't propose merging already-stale candidates.

### 3.2 Curator-agent

New agent: `agents/curator-agent/` with `AGENTS.md` (system prompt) and `config.yaml` (provider/model null → orchestrator inherits per gap #3 / per-ward config pattern just shipped).

System prompt (outline — full text under `gateway/templates/agents/curator-agent.md`):

> You are the **ward curator**. You see a table of agent-authored wards: name, Purpose/Scope blurb, use_count, last_used_at, age_days, state. Cluster wards whose Purpose/Scope materially overlap. For each cluster of ≥2 candidates, decide one action:
>
> - **MERGE** — combine into a new umbrella ward whose Purpose/Scope is the union of inputs. Specify the umbrella name.
> - **ABSORB** — move sibling content into an existing umbrella in the cluster, archive the sibling.
> - **LEAVE_ALONE** — distinct enough; no action.
>
> Emit a fenced YAML block:
>
> ```yaml
> consolidations:
>   - from: [ward-a, ward-b]
>     into: umbrella-c
>     action: merge
>     reason: "both target vehicle-history research; merging into a single domain"
> prunings:
>   - ward: orphan-x
>     reason: "no activity in 47d AND no procedures"
> ```
>
> Hard rules: never touch `created_by != "agent"`, never touch `pinned: true`, never delete (archive only — every action is recoverable), `use_count == 0` is not a consolidation signal, every `from` ward must have ≥1 successful use in the last 60d to even be a merge candidate (otherwise it's a Phase B archival).

### 3.3 `ward_manage` tool

A new tool exposed **only** to the curator-agent (gated by a task-local marker in `agent_runtime` — analogous to Hermes's `ContextVar`):

| Action | Semantics |
| --- | --- |
| `ward_manage(action="merge", from=[a,b], into="umbrella", purpose="...")` | Creates umbrella with combined Purpose/Scope, copies each `from`'s `memory-bank/`, `specs/`, and procedures (re-keyed by ward_id) into umbrella, then archives each `from` with `absorbed_into="umbrella"` in its sidecar entry. |
| `ward_manage(action="absorb", from="a", into="b")` | Same as merge but `into` already exists; doctrine of `into` is amended. |
| `ward_manage(action="archive", ward="x", reason="...")` | Equivalent to Phase B archive, but invokable from the curator-agent. |

Procedure copy uses `ProcedureRepository`: `list_by_ward(from) → upsert with ward_id = into`. KG/episode re-keying is **deferred** to a v2 — for v1 we keep the original ward's KG entries reachable via the umbrella's reference to `_archive/<from>/`.

### 3.4 Safety

- Identical backup contract to Phase B (tar.gz pre-run).
- `dry_run = true` is the default — returns the YAML plan without mutating.
- Hard cap of `max_consolidations = 5` per run.
- LLM client: orchestrator config for v1 (auxiliary slot when gap #3 lands).
- Audit log mirrors Phase B's, **plus** the curator-agent's full tool-call trace.

---

## 4. Configuration

`settings.json` gains:

```json
{
  "ward_curator": {
    "enabled": true,
    "stale_days": 30,
    "archive_days": 90,
    "backup_keep": 5,
    "consolidation_dry_run_default": true,
    "consolidation_max_per_run": 5
  }
}
```

`POST /api/wards/curator/pause` + `POST /api/wards/curator/resume` toggle `paused` at runtime without editing settings.

---

## 5. Tests

| Layer | Test |
| --- | --- |
| `WardUsage` | atomic write round-trip; concurrent bumps from N threads don't lose updates |
| `bump_use` | wired in `spawn.rs` — table-driven test simulating ward delegations |
| Layer-1 algorithm | every branch (`active→stale`, `stale→archive`, `stale→active`, pinned skip, non-agent skip) |
| Backup | round-trip restore returns the original tree byte-for-byte |
| Cleanup endpoint | dry-run returns the plan and mutates nothing; live run is idempotent (second call is a no-op) |
| Restore endpoint | round-trip after a destructive change |
| Curator-agent prompt | snapshot test on a fixed ward table produces a well-formed YAML block |

---

## 6. Out of scope (explicit)

- Operating on user-authored or bundled wards.
- Failure-log mining (Hermes doesn't, neither do we).
- An auxiliary LLM slot (covered by gap #3 — smart per-task routing).
- A direct `kind: "http"` cron action type (worth doing — but separately).
- KG/episode re-keying on merge (v2).

---

## 7. Implementation order

1. **A.1** `WardUsage` service + sidecar schema + atomic write
2. **A.2** Bump points (delegation, write_file/edit_file, ward creation, boot seed)
3. **A.3** `GET /api/wards/usage[/<ward>]` + `POST /api/wards/usage/<ward>/pin`
4. **B.1** Layer-1 algorithm (pure Rust, no endpoint yet)
5. **B.2** Backup writer + audit log writer
6. **B.3** `POST /api/wards/curator/cleanup` (the headline B deliverable)
7. **B.4** `POST /api/wards/curator/restore` + default cron entry
8. **C.1** `ward_manage` tool surface (with task-local gate)
9. **C.2** `curator-agent` definition (`agents/curator-agent/AGENTS.md` + `config.yaml`)
10. **C.3** `POST /api/wards/curator/consolidate`

A and B are independently shippable. C requires A, and runs cleaner after B.
