# Implementation review — round 1

## Blocker

**Matching-dimension malformed tables could be skipped.**

The initial skip decision parsed the first `FLOAT[N]` occurrence without
proving that the live object was a `vec0` virtual table. An ordinary table
declaring `embedding FLOAT[384]` could therefore masquerade as healthy.

## Resolution

`read_table_dim` now rejects DDL unless it represents
`CREATE VIRTUAL TABLE ... USING vec0`. The malformed-table regression uses an
ordinary table with a matching `FLOAT[384]` declaration, proving that this
case rebuilds rather than skips.
