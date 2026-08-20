---
"@smooai/clickhouse-kit": minor
---

TS + Zod code emit in the Rust crate (`crates/clickhouse-kit/src/codegen.rs`) — from a `TableSpec`, emit a TS row `interface`, a Zod **select** schema, and a Zod **insert** schema (columns with a ClickHouse `DEFAULT` become `.optional()`), for schema/consumer parity with `postgres-kit`. Always compiled; there is no cargo feature gating it. Mirrors the retired TS package's `createSelectSchema`/`createInsertSchema` output style: `camelCase` keys, 4-space formatting, and the same ClickHouse→TS/Zod type mapping (`String`/`UUID`/dates→`string`/`z.string()`, ints/floats→`number`/`z.number()`, `Bool`→`boolean`, `Array(String)`→`string[]`/`z.array(z.string())`, `Map(String,String)`→`Record<string,string>`/`z.record(z.string(),z.string())`, `JSON`→`unknown`/`z.unknown()`, `Nullable(T)`→optional `T | null`/`.nullable()`, `LowCardinality(T)` transparent → `T`).
