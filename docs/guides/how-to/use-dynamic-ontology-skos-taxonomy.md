# Use Dynamic Ontology and SKOS Taxonomy

This guide covers the zbot governance layer on top of Engram memory. Ontology
governs entity classes, relationship predicates, and advisory validation. SKOS
taxonomy governs controlled concepts, labels, direct broader/narrower/related
relations, and bounded recall expansion.

## Defaults

Fresh installs are inert:

- no ontology definition path is configured
- no taxonomy definition path is configured
- validation mode is `advisory`
- unclassified records are allowed
- existing gateway, UI, WebSocket, store-trait, and Observatory payloads keep
  their old shape, with optional additive governance health fields

When governance is configured, zbot owns selection and policy. Engram stays the
generic storage/query framework for ontology and taxonomy records.

## Configure Governance

Edit `~/Documents/zbot/config/settings.json` and add governance under
`execution.memory.provider`:

```json
{
  "execution": {
    "memory": {
      "provider": {
        "governance": {
          "ontologyDefinitionPaths": ["governance/base-ontology.json"],
          "taxonomyDefinitionPaths": ["governance/base-taxonomy.json"],
          "defaultSelection": {
            "ontologyIds": ["zbot.base:v1"],
            "taxonomySchemeIds": ["zbot.general:v1"]
          },
          "overlays": [
            {
              "wardId": "research",
              "selection": {
                "ontologyIds": ["zbot.base:v1"],
                "taxonomySchemeIds": ["zbot.general:v1"]
              }
            }
          ],
          "validationMode": "advisory",
          "allowUnclassified": "warn",
          "skosExpansion": {
            "maxDepth": 1,
            "maxFanOut": 8,
            "maxCandidates": 8
          }
        }
      }
    }
  }
}
```

Definition paths must stay under `~/Documents/zbot/config/`. They are resolved,
confined, and fingerprinted for migration safety. In the current implementation,
startup bootstraps the built-in `zbot.base:v1` ontology and
`zbot.general:v1` SKOS scheme; local definition file parsing/import is tracked
as follow-up work.

Selection precedence is:

1. task
2. source
3. session
4. project
5. ward
6. default selection

Validation is advisory only. Unknown or mismatched predicates record sanitized
findings; they do not block writes.

## Operate

Use these read models to confirm behavior:

- `GET /api/graph/stats` includes optional `governance`
- `GET /api/memory/health` includes optional `governance`
- recall traces include non-secret taxonomy expansion cues when SKOS expansion
  widens the query
- migration dry-run manifests include governance selection, validation mode,
  allow-unclassified policy, SKOS expansion limits, and a path-free governance
  fingerprint

The governance health block exposes support state, selected IDs, bootstrap
counts, policy settings, finding count, and sanitized finding codes only. It
does not expose raw transcripts, absolute paths, SQL, embeddings, or secrets.

## Roll Back

To disable governance behavior:

1. Remove `execution.memory.provider.governance` from
   `~/Documents/zbot/config/settings.json`, or clear `defaultSelection`,
   `overlays`, `ontologyDefinitionPaths`, and `taxonomyDefinitionPaths`.
2. Restart the daemon.
3. Confirm `/api/graph/stats` and `/api/memory/health` either omit governance
   or report unsupported/inactive governance.

Existing unclassified memory and graph records remain readable.

## Current Gaps

- Local ontology/taxonomy definition files are confined and fingerprinted, but
  not imported as active definitions yet.
- Governance validation findings are stored in the zbot adapter sidecar until
  Engram exposes a generic validation-finding read port.
- There is no UI editor or auto-merge path for model-discovered ontology terms
  or taxonomy concepts. That is intentional; activation needs a future governed
  merge policy.
