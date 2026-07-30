# Research archetype contract

Research-producing skills share this workflow while retaining their own
activation triggers and domain methods.

## Placement

- Treat the Active Ward Template as the only layout and format authority.
- Use exact task paths only when the template permits them.
- Do not assume a research root, dated snapshot directory, index filename,
  document format, archive, or auxiliary knowledge vault.
- If no declared rule can hold a requested persistent artifact, keep that
  result ephemeral and return `role_not_declared` with the template digest.

## Workflow

1. Search the active ward and injected graph/context for prior work on the
   subject.
2. Gather evidence using the archetype's approved capabilities.
3. Preserve source provenance and distinguish current observations from durable
   findings.
4. Write only task-specified or template-declared artifacts. Use the declared
   format and required metadata rather than a fixed heading or frontmatter
   schema.
5. When the template declares an overview/index role, link it to the created
   artifacts using the declared format's link mechanism. Otherwise return a
   response-level inventory.
6. When graph ingestion is requested and available, submit one bounded summary
   entity plus evidence-backed cross-source entities and relationships.
7. Save memory only for durable findings likely to matter across sessions; omit
   ephemeral snapshot values.

## Retrieval and retention

Resolve previous artifacts through ward search, returned paths, and graph
properties. Never reconstruct a path from a remembered convention. Preserve
prior snapshots unless the user explicitly requests replacement or cleanup.

## Boundaries

- Do not create entity-page trees unless the active template declares them.
- Do not emit auxiliary graph files unless the task/template requests them.
- Do not rename existing artifacts merely to match a preferred layout.
- Do not edit ward instructions or infrastructure unless explicitly assigned.
