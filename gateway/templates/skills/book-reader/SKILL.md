---
name: book-reader
description: Read, identify, chunk, analyze, and recall long-form documents such as TXT, Markdown, EPUB, and PDF. Use when a user wants a book or long document processed progressively, summarized by section, indexed for later retrieval, or represented in the knowledge graph.
---

# Book Reader

Process long-form documents progressively. Never load an entire book into
working context when bounded section reads are possible.

## Output placement

1. Use an exact output path supplied by the task when it is allowed by the
   Active Ward Template.
2. Otherwise inspect the Active Ward Template and use only declared paths and
   formats. In a root context, preview a suitable `createConcept` operation
   with `ward(action="dry_run", operation="create_concept", ...)` before
   creating it. In a delegated context, use a preview supplied by the root or
   return a bounded request containing the proposed operation and inputs.
3. Treat the root-supplied dry-run result as the file inventory. Do not invent
   conventional folders, filenames, Markdown-only outputs, indexes, chapter
   directories, or entity directories.
4. If the template cannot represent the requested durable artifacts, write
   nothing and return `role_not_declared` with the template digest and the
   ephemeral reading result.

## Workflow

### 1. Identify the source

Extract title, author, language, publication metadata, and stable source
identity from the document itself rather than its filename.

- For EPUB, follow `references/epub.md`.
- For PDF, follow `references/pdf.md`.
- For TXT or Markdown, follow `references/txt.md`.

If the title cannot be established, report the missing metadata instead of
guessing.

### 2. Check prior knowledge

Search the active ward by title, aliases, author, tags, and known identifiers.
Use injected knowledge-graph context when present. Reuse an existing concept
only when the evidence identifies the same work.

### 3. Build a reading skeleton

Follow `references/chunking.md` to identify ordered chapters or bounded
sections. Preserve source order and stable line/page/section provenance.

### 4. Read progressively

For each section:

- retain the exact source boundary;
- summarize the section;
- extract important claims, entities, relationships, events, themes, quotes,
  and open questions;
- preserve citations back to the source boundary;
- write only artifacts present in the approved template/task inventory.

Do not force a fixed heading vocabulary. Use the declared document format and
the structure appropriate to the source and request.

### 5. Build retrieval pointers

When the declared inventory includes an overview or index, connect it to the
created section and entity artifacts using the link mechanism appropriate to
the declared format. Verify every target exists. When no overview is declared,
return the inventory in the response instead.

### 6. Record durable knowledge

When the `ingest` tool is available and the task calls for durable recall,
submit one bounded batch containing:

- one summary entity for the work;
- real cross-source entities worth reconciling across documents;
- evidence-bearing relationships supported by section provenance.

Keep fictional or source-local entities in declared ward artifacts unless the
task explicitly requests graph ingestion. Save only durable memory facts; omit
ephemeral per-run measurements.

## Retrieval behavior

For a previously processed work, search the active ward and injected graph
context before reading the source again. Resolve paths from returned results,
not remembered layouts. Do not re-ingest or re-chunk unless the user requests a
refresh or the source identity/version changed.

## Completion checks

- Every created path appears in the task or Active Ward Template preview.
- Every declared required artifact exists and uses its declared format.
- Every link or relationship target resolves.
- Every factual claim and quote retains source provenance.
- No metadata was inferred from the filename when source metadata was
  available.
- The response lists created paths, skipped roles, and the template digest.
