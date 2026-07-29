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

- Ward creation now supports explicit `generic` and `coding` archetypes from a
  local, editable registry. Existing singular Ward templates transition
  non-destructively into `generic`, while created Wards retain their copied
  layout and recorded archetype.
- First-time commissioning now offers an explicit recommended Full Zbot memory
  profile with built-in embeddings, pinned recall/governance configuration, and
  a restart-verified activation screen. The safe baseline remains available
  and preserves existing memory configuration.

### Changed

- New Wards for every bundled archetype now start with one Ward-named canonical
  page, agent instructions, an append-only log, and their layout snapshot.
  Specialized work folders, linked pages, sources, and
  `.zbot/specs/<concept>/` planning files remain lazy.
- `wards/index.md` is now the sole index and catalogs each Ward with a
  canonical wikilink. Ward Markdown uses the filesystem-authoritative LLM Wiki
  model without mandatory OKF frontmatter.
- Journal Ward templates now route daily material to one
  `entries/YYYY/YYYY-MM-DD.md` file per source day and reject loose Markdown
  directly under `entries/`, avoiding accidental single-document compilation.

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
