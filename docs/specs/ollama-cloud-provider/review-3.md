# Review 3

## Blockers

**1. Recovery/concurrency policy lacked direct status and lock tests.** Fixed by
adding regressions for an old Complete status with an active marker, dangling
markers, and concurrent ownership of the shared provider mutation lock.

**2. Provider request and response contracts remained partially inconsistent.**
Fixed by aligning TypeScript with the server's full-replacement update contract,
documenting optional provider fields, and declaring update/default response
bodies in OpenAPI.

## Concerns

**3. Pending-marker inspection was duplicated.** Fixed by moving the marker
name and fail-closed inspection helper into `gateway-services` and reusing it
from commissioning and provider routes.

**4. Redirect hardening changed non-Ollama verification behavior.** Fixed by
restoring the existing redirect behavior for other providers and applying the
no-redirect policy only to the fixed Ollama Cloud identity.

