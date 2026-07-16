## Concerns

**1. Ordinary relation evaluator remains unowned.** `docs/specs/autonomy-ledger/plan.md:89`. The plan promised `new`/`related`/`ambiguous` for ordinary requests but gave no resolver owner, matching rule, or surface. Fix: remove the unimplemented reporting promise and retain the explicit-selection, no-implicit-attachment guarantee.
