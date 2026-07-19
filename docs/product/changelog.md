# Changelog

All notable user-visible changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> Maintenance: this file is updated in the same PR that introduces the
> change. CI will warn (configurable: block) when a PR touches code that
> changes user-visible behavior but does not touch this file.
>
> Entries can be drafted from conventional commits: `git log --oneline`
> filtered to `feat:` and `fix:` since the last tag is a starting point,
> not a finished product. Rewrite for users, not contributors. See the
> [Common Changelog guidance](https://common-changelog.org/) — the audience
> is humans who use the software, not humans who wrote it.

## [Unreleased]

### Added

- First-time commissioning now offers an explicit recommended Full Zbot memory
  profile with built-in embeddings, pinned recall/governance configuration, and
  a restart-verified activation screen. The safe baseline remains available
  and preserves existing memory configuration.

### Changed

- (nothing yet)

### Deprecated

- (nothing yet)

### Removed

- (nothing yet)

### Fixed

- (nothing yet)

### Security

- Commissioning completion now rejects non-local originless peers before body
  parsing and provisions fixed memory-profile files without following symlinks
  or overwriting conflicting content.
