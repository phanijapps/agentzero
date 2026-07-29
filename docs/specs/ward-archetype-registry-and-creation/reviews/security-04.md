## Concerns

1. **[hybrid] Sidecar reads still follow unbounded unsafe paths.** Load the
   sidecar through the verified wards directory, reject symlink, hard-link,
   FIFO, and non-regular inputs, enforce a byte cap, and treat unsafe,
   unreadable, or malformed data as empty with a bounded warning.
