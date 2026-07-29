# RFC-0016 layout-resolution spike

## Hypothesis

One constrained, generic, data-only ward layout can resolve the required roles
for unrelated and nested domain concepts without domain profiles or executable
templates. The shared catalog can resolve each ward's exported index role even
when a ward customizes that path.

## Reproduce

From the repository root:

```bash
python3 docs/rfc/0016-notes/layout_resolution_spike.py
```

The script uses the RFC's `{conceptPath}`/`{conceptName}` model and resolves
the concept root, canonical document, spec, plan, task document, run document,
and every required intermediate index. It also checks all four resource areas,
identifier components, collisions, and a catalog entry for a ward whose index
role has changed from `index.md` to `home.md`.

For every rendered path, it asserts that the path is relative and normalized,
contains no `.` or `..` component, and does not collide with another resolved
role.

## Result

```text
aapl-analysis: 13 unique roles
great-expectations: 13 unique roles
great-expectations/chapter-01: 13 unique roles
resources: 8 unique roles
catalog targets: {'default-ward': 'default-ward/index.md', 'custom-ward': 'custom-ward/home.md'}
spike: PASS
```

The spike establishes adequate expressiveness for unrelated and recursive
concept shapes. It does not establish filesystem/symlink safety; that is a
separate interpreter requirement in RFC-0016.
