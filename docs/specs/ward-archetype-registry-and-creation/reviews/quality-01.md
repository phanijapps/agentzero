## Blockers

1. **Creation can publish a Ward without durable archetype provenance.**
   Publishing precedes the sidecar update, so a persistence failure can leave a
   visible Ward without its required archetype record. Use the runner's shared
   `WardUsage` service, compensate by rolling back the published Ward, and test
   an unwritable sidecar.

2. **The clean-start E2E acceptance test is not executed by CI.** The existing
   job only discovers the UI package's suite. Install and invoke the separate
   `e2e/playwright` suite explicitly and retain its failure report.
