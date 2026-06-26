---
"@smooai/clickhouse-kit": patch
---

Add `ch.nullable(inner)` (renders `Nullable(<inner>)`, composes with `lowCardinality` for `LowCardinality(Nullable(String))`) and `ch.json()` (the native ClickHouse `JSON` type) column helpers.
