## Concerns

1. **[hybrid] Archetype seeding bypasses the confined no-follow filesystem
   helpers.** A local attacker or compromised tool with access to the vault can
   race a checked archetype directory to a symlink before the seed file is
   opened. Use the directory-fd-based helpers on Linux and canonicalize after
   each creation on portable targets.

2. **[reason] Linux archetype resolution does not reject case-fold aliases.**
   Require exactly one byte-exact entry and reject case-insensitive siblings
   before resolving every registry component.

3. **[reason] Doctrine injection filtering is an incomplete denylist.** Reject
   normalized role labels, instruction-reset verbs, and privileged prompt
   references rather than matching only a few literal phrases.

## Not Checked

Dependency and CI vulnerability scanning were not run; those remain owned by
SCA tooling such as Dependabot or `cargo audit`.
