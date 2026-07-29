---
name: html-report
description: Create polished self-contained HTML reports, dashboards, and data visualizations from supplied evidence. Use when a user requests an HTML report, styled analytical document, dashboard, scenario analysis, portfolio report, or other browser-rendered deliverable.
---

# HTML Report

Create a self-contained HTML artifact from supplied evidence and the bundled
visual templates.

## Output placement

- Use the exact output path from the task when the Active Ward Template allows
  it.
- Otherwise resolve a declared resource area or file rule that permits HTML or
  raw output from the Active Ward Template. Never assume a report directory or
  filename.
- If no suitable rule exists, write nothing and return `role_not_declared` with
  the template digest.
- Treat paths shown inside bundled templates as examples, never destinations.

## Choose a bundled template

- `template.html` — general dashboard or analytical report.
- `portfolio-template.html` — portfolio overview.
- `pnl-template.html` — profit-and-loss reconciliation.
- `trade-specification-template.html` — trade specification.
- `risk-reversal-template.html` — risk-reversal analysis.
- `cri-template.html` — CRI scan.
- `stress-test-template.html` — scenario stress testing.

Read `THEME.md` for the visual system. Reuse the closest template rather than
rebuilding its shell.

## Build

1. Read the actual input artifacts named by the task.
2. Select the closest template and preserve its responsive layout, typography,
   and component conventions.
3. Replace example values with evidence-backed values. Remove unused example
   sections and controls.
4. Keep CSS and JavaScript inside the output unless the task and active
   template explicitly declare companion assets.
5. Escape untrusted text before inserting it into HTML or JavaScript contexts.
6. Include provenance near material numbers, charts, and conclusions.
7. Match the report structure to the request and evidence; do not impose a
   finance-specific outline on unrelated work.

## Verify

- Open or render the produced file and check visible layout, overflow, labels,
  and empty states.
- Confirm charts and calculations match their source inputs.
- Confirm no placeholder values, broken assets, secrets, or host paths remain.
- Confirm the output path is task-specified or template-declared.
- Return the created path, a concise verification result, and the template
  digest.
