# Security boundaries

AgentZero treats model output, tool output, connector data, and persisted
conversation content as untrusted input. Features that project this data into a
user interface must preserve the following controls.

## Agent-driven surfaces

The A2UI work-surface catalog is a declarative display boundary, not an
execution boundary.

- The portable Rust catalog is the publication gate. It accepts only known
  component types and component-specific properties, validates JSON-pointer
  bindings, rejects oversized surface payloads, and applies the documented
  collection, field, and rendered-string budgets to expanded components.
- New display components must not accept HTML, CSS, URLs, code, formatter
  functions, event handlers, or actions. Write-capable behavior requires a
  separately specified and server-allowlisted gateway action.
- React renderers resolve bindings against the supplied data and render values
  as text nodes. Missing, malformed, non-finite, or wrong-shaped values fail
  locally to an empty state so one component cannot prevent its siblings from
  rendering.
- Chart configuration is renderer-owned. Agent-authored data can select only
  validated field keys and bounded series; it cannot provide Recharts
  components, callbacks, styles, or markup.
- Canonical text and artifacts remain authoritative. Surface rendering is
  optional and cannot gate execution completion.

The normative surface contract is
[`contracts/asyncapi/agent-surfaces.yaml`](../../contracts/asyncapi/agent-surfaces.yaml).
Feature-specific limits and verification live in the linked specification
under `docs/specs/`.

## Frontend dependency intake

New runtime dependencies require explicit approval and a lockfile update.
Before delivery:

- run the package manager's production-dependency vulnerability audit;
- confirm newly reported advisories are not introduced through the dependency;
- run the full UI test suite and production build;
- keep dependencies declarative at the untrusted-data boundary, without
  enabling evaluation of model-authored code or markup.

Existing unrelated advisories are recorded in the implementation handoff and
handled through their owning upgrade work rather than silently broadened into
the feature change.
