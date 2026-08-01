# Implementation review — round 3

## Blocker

**A vec0 table with the expected columns plus extra schema could still be
skipped.**

The shape check found the required definitions but did not reject additional
column definitions.

## Resolution

The live-table check now requires exactly the two canonical vec0 definitions.
The malformed integration regression constructs a matching-dimension table
with both expected columns plus an unexpected vector column and proves that it
is rebuilt.
