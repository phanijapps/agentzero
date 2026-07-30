## Blockers

1. **Windows sidecar updates fail once `.usage.json` already exists.** Use an
   atomic replace-existing primitive on Windows and add a portable test that
   persists multiple updates to the same sidecar.
