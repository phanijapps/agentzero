# Review 2

## Outcome

Not clean. The adversarial, security, and quality reviewers found recovery,
contract, filesystem, origin, and response-boundary gaps.

## Blockers

**1. Stale Complete state could erase an active recovery marker.**
   Fixed by making successful commissioning persistence the only marker-removal
   path; status always reports any marker as recovery pending.

**2. Provider OpenAPI did not match the camelCase redacted API.** Fixed
   by documenting the actual public response and create/update request shapes,
   including the 201 create response and credential-retention semantics.

**3. Dangling marker symlinks bypassed pending checks.** Fixed by using
   `symlink_metadata` and treating every entry or inspection error except
   NotFound as pending. Added a dangling-symlink regression test.

**4. Fixed-origin and error-body controls had gaps.** Fixed by enforcing
   the exact Ollama Cloud base URL, disabling redirects for every provider
   verification request, and bounding inference/multimodal error bodies.

## Concerns

**5. Recovery and concurrency coverage was incomplete.** Expanded
   exact-marker conflict coverage; the shared mutation lock and marker lifecycle
   are covered by focused service/commissioning tests. Full workspace and UI
   suites remain required before shipment.
