# @smooai/clickhouse-kit

## 0.1.1

### Patch Changes

- fc2dec5: Add `ch.nullable(inner)` (renders `Nullable(<inner>)`, composes with `lowCardinality` for `LowCardinality(Nullable(String))`) and `ch.json()` (the native ClickHouse `JSON` type) column helpers.
