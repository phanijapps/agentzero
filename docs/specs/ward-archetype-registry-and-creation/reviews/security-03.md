## Concerns

1. **[hybrid] Provenance sidecar writes still follow attacker-controlled
   temp-file links.** Replace the fixed `.usage.json.tmp` write with a unique,
   exclusive, no-follow, single-link temporary file beneath a verified real
   wards directory; fsync it and atomically rename it into place.
