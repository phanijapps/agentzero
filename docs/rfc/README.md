# RFCs

| RFC | Status | Summary |
| --- | --- | --- |
| [RFC-0004: GitHub Release Installer and Cross-Platform Packaging](0004-github-release-installer-and-packaging.md) | Draft | Proposes GitHub-hosted installers that download verified GitHub Release artifacts for Linux, macOS, and Windows. |
| [RFC-0005: Builder Delegation and Ward Context Hygiene](0005-builder-delegation-and-ward-context-hygiene.md) | Accepted | Adds delegation modes so builder can distinguish direct artifacts, ward hygiene, ward-backed builds, and step execution. |
| [RFC-0006: Vault Obsidian-Style Ward Browser](0006-vault-obsidian-style-ward-browser.md) | Draft | Proposes a top-level Vault tab for browsing ward filesystem trees and previewing common files in-app. |
| [RFC-0007: Ward-Specific Vault Explorer in Research](0007-ward-specific-vault-in-research.md) | Open | Proposes embedding a ward-scoped Vault explorer/search pane inside Research once a session has an active ward. |
| [RFC-0008: Simplified Provider Model Configuration](0008-simplified-provider-model-configuration.md) | Accepted | Replaces broad maintained model metadata with simple 200k input / 32k output defaults plus agent and Advanced overrides. |
| [RFC-0009: Agent Runtime Budget and Retry Governance](0009-agent-runtime-budget-and-retry-governance.md) | Draft | Proposes prompt-safe tool-result persistence, token-budgeted continuation history, and bounded delegation retries for long sessions. |
| [RFC-0010: Extract Runtime Compaction Crate](0010-extract-runtime-compaction-crate.md) | Draft | Proposes extracting runtime compaction boundaries while preserving durable memory ownership and migration constraints. |
| [RFC-0011: Engram Memory Engine Cutover](0011-engram-memory-engine-cutover.md) | Draft | Proposes cutting zbot over to Engram through a zbot-owned adapter while keeping gateway, UI, settings, and sleep-cycle contracts stable. |
| [RFC-0012: Engram Upstream Risk Reduction](0012-engram-upstream-risk-reduction.md) | Draft | Tracks generic upstream Engram framework dependencies that reduce AgentZero cutover risk without moving zbot-specific contracts into Engram. |
| [RFC-0014: Context Capability Registry and Context Graph](0014-context-capability-registry-and-context-graph.md) | Draft | Proposes a zbot-owned capability registry, resource/context packet model, ingestion boundary, and staged tool-surface cleanup. |
