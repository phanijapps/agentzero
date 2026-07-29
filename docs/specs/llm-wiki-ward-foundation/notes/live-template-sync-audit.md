# Live template synchronization audit

- **Run date:** 2026-07-28
- **Repository registry:** `gateway/templates/wards`
- **Live registry:** `/home/videogamer/Documents/zbot/config/templates/wards`
- **Primary create-new backup:**
  `/home/videogamer/Documents/zbot/config/templates/wards.backup-20260728T154708Z`
- **Intermediate create-new backup:**
  `/home/videogamer/Documents/zbot/config/templates/wards.backup-20260728T154815Z`

## Result

| Check | Before | After | Result |
| --- | --- | --- | --- |
| Existing-Ward normalized recursive manifest | `c398ed23bb9ad5ec1a5f03da9e2723a44be498de2e0921f7e10a92b958bb08a6` | `c398ed23bb9ad5ec1a5f03da9e2723a44be498de2e0921f7e10a92b958bb08a6` | unchanged |
| Repository/live template normalized recursive manifest | repository `eef2d7545f48261342adb4aa2fc7f41df96053c28f73fe238909b14886600dad` | live `eef2d7545f48261342adb4aa2fc7f41df96053c28f73fe238909b14886600dad` | identical |

The normalized recursive manifests include relative paths, entry types, and
regular-file content digests. A second independent files-only audit after the
test run also matched repository and live registries at
`8f07e103fae6366902a07b87492797360b9fb1f88e2de470813fea5ea28270c0`.

## Safety preflight

- Resolved every live-registry ancestor with `namei`; no ancestor was a
  symlink.
- Rejected proceeding if the selected backup or staging destination already
  existed; both backup names above were created with no-replace semantics.
- Audited the repository source and live destination for symlinks, special
  files, case-fold path collisions, and regular files with more than one hard
  link; none were present.
- Staged the complete registry before replacement and compared its normalized
  manifest with the repository registry.
- Rechecked the live registry after publication and rechecked the existing-Ward
  manifest. No existing Ward was created, removed, or modified.

The primary backup contains the live registry as it existed immediately before
the clean-break synchronization and is the recovery point for this rollout.
