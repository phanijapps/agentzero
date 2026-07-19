# Review 4

## Blockers

**1. Correcting a hand-edited invalid cloud origin could erase a retained key.**
Fixed by retaining the stored key within `ProviderService::update` before origin
validation, with a regression covering correction from an invalid stored URL.

## Concerns

**2. Redirect policy required an explicit cross-provider decision.** Resolved in
favor of credential safety: provider verification never follows redirects, and
the spec/plan now identify that behavior as an intentional cross-provider
security correction. Ollama Cloud additionally enforces its exact fixed origin.

