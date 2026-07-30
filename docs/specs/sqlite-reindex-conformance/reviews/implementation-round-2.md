# Implementation review — round 2

## Blocker

**A malformed vec0 table could still be skipped when an unexpected vector
column had the requested dimension.**

The first repair proved the table used vec0 but did not prove that the parsed
dimension belonged to the target's expected embedding column.

## Resolution

The live-table check now validates the complete target shape before skipping:
the expected text primary-key column must exist, and the requested dimension
must belong to the expected embedding column. Regressions cover an ordinary
lookalike table, missing expected columns, and a vec0 table whose unexpected
vector column has the matching dimension.
