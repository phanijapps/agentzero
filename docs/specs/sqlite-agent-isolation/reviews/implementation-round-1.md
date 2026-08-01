# Implementation review — round 1

## Quality concern

**1. Conformance test could pass without proving positive visibility.**

The scope allowlist rejected unrelated agent identifiers but could pass for an
empty result. Require both the requested-agent fixture and an explicitly
global fixture to be present.

## Resolution

The conformance scenario now seeds `SharedAcrossAgents` under `__global__` and
asserts both it and `OnlyA` are returned before applying the scope allowlist.
