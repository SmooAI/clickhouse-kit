# smooai-clickhouse-kit

[![crates.io](https://img.shields.io/crates/v/smooai-clickhouse-kit.svg)](https://crates.io/crates/smooai-clickhouse-kit)
[![docs.rs](https://img.shields.io/docsrs/smooai-clickhouse-kit)](https://docs.rs/smooai-clickhouse-kit)
[![CI](https://github.com/SmooAI/clickhouse-kit/actions/workflows/rust.yml/badge.svg)](https://github.com/SmooAI/clickhouse-kit/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

**A safe-by-construction schema toolkit for ClickHouse — for user-defined, multi-tenant schemas, with a TypeScript→Rust bridge for the schemas you author by hand.**

The kit has two jobs:

1. **Runtime toolkit (user-defined / multi-tenant tables).** When your customers' data shapes are defined at runtime, you end up turning untrusted input into SQL. The kit owns that boundary so the happy path makes **SQL injection and unbounded tables impossible, not merely discouraged** — an allowlisted type system, identifier validation, DDL generation, `flexible_table`, forward-only migrations, and additive evolution.
2. **TS→Rust bridge (developer-authored tables).** When TypeScript owns a table's schema, `introspect` reads the live ClickHouse back into Rust and `codegen` emits the `#[derive(Row)]` struct, with `check_drift` asserting the Rust view ≡ the live DB. No more hand-copied row structs drifting from the schema.

Either way, rows stay [Serde](https://serde.rs)-native (use the [`clickhouse`](https://crates.io/crates/clickhouse) crate's `#[derive(Row)]`) — the kit never reimplements row mapping.

```toml
[dependencies]
smooai-clickhouse-kit = "0.1"
```

> The crate is `smooai-clickhouse-kit`; it imports as **`clickhouse_kit`** — `use clickhouse_kit::...`.

## Turn untrusted input into safe DDL

A column type can come straight from a customer config / JSON. The allowlist is an **enum** — disallowed types like `Decimal`, `FixedString`, `Tuple`, or arbitrary expressions simply have no representation, so they fail to deserialize at the boundary. There is no path to an arbitrary type string reaching the DDL.

```rust
use clickhouse_kit::{
    to_create_table_sql, ColumnSpec, ColumnTypeSpec, ScalarType, SchemaLimits, TableSpec,
};

// `{"lowCardinality": "String"}` from untrusted JSON — `Decimal(...)` here would be rejected.
let org_type: ColumnTypeSpec = serde_json::from_str(r#"{"lowCardinality":"String"}"#)?;

let table = TableSpec {
    name: "events".into(),
    columns: vec![
        ColumnSpec { name: "id".into(),  type_spec: ColumnTypeSpec::Scalar(ScalarType::Uuid),       default: None },
        ColumnSpec { name: "org".into(), type_spec: org_type,                                       default: None },
        ColumnSpec { name: "ts".into(),  type_spec: ColumnTypeSpec::Scalar(ScalarType::DateTime64), default: None },
    ],
    engine: "MergeTree()".into(),
    order_by: vec!["id".into()],
};

let ddl = to_create_table_sql(&table, &SchemaLimits::default())?;
// CREATE TABLE IF NOT EXISTS events (
//     id UUID,
//     org LowCardinality(String),
//     ts DateTime64(3)
// )
// ENGINE = MergeTree()
// ORDER BY (id)
```

Every identifier is validated (`^[A-Za-z_][A-Za-z0-9_]*$` + a length bound, backtick-quoted on render), column counts are bounded, and `ORDER BY` entries must be real columns — so a malicious table/column name can't inject SQL.

## The flexible (hybrid) table

The most-reused multi-tenant shape in one call — your mandatory + promoted typed columns, plus an `attrs Map(String, String)` catch-all and a `raw String`:

```rust
use clickhouse_kit::{flexible_table, FlexibleConfig, ColumnSpec, ColumnTypeSpec, ScalarType, SchemaLimits};

let table = flexible_table(
    "customer_events",
    FlexibleConfig {
        mandatory: vec![ColumnSpec { name: "ts".into(), type_spec: ColumnTypeSpec::Scalar(ScalarType::DateTime64), default: None }],
        promoted:  vec![ColumnSpec { name: "amount".into(), type_spec: ColumnTypeSpec::Scalar(ScalarType::Float64), default: None }],
        engine: "MergeTree()".into(),
        order_by: vec!["ts".into()],
        reserved: None, // defaults to ["attrs", "raw"]
    },
    &SchemaLimits::default(),
)?;
```

## Ingest: flatten + coerce

Shape an arbitrary record to a (possibly dynamic) table — known keys land in their columns, the long tail flattens into `attrs`, and `raw` captures the original:

```rust
use clickhouse_kit::{coerce_to_table, FlattenOptions};

let result = coerce_to_table(input_json, &table, &FlattenOptions::default());
// result.row: BTreeMap<String, Value> ready to insert · result.overflow_keys: what went to `attrs`
```

## Migrations + drift — bring your own client

The I/O layer is written against a tiny `ChExecutor` trait, so the crate never depends on a concrete ClickHouse driver. Implement it over the [`clickhouse`](https://crates.io/crates/clickhouse) crate (or any client):

```rust
use clickhouse_kit::{run_migrations, check_drift};

// forward-only, tracked in `_ch_migrations`; already-applied files are skipped
let applied = run_migrations(&exec, std::path::Path::new("clickhouse/migrations")).await?;

// compare the live schema (system.columns) to your TableSpecs
let drift = check_drift(&exec, &[table]).await?;
```

For growing a per-tenant table, `diff_columns` + `alter_add_columns_sql` emit a guarded, **additive-only** `ALTER TABLE … ADD COLUMN IF NOT EXISTS …` (identifiers quoted; types from your trusted spec, never from the live DB).

## TS→Rust bridge: generate Rust rows from a TS-authored table

When the schema lives in TypeScript, you don't hand-write (and re-sync) the Rust row struct — introspect the live table and generate it:

```rust
use clickhouse_kit::introspect_row_struct;

// Reads system.columns for `events` and emits the Rust source:
let src = introspect_row_struct(&exec, "events", "EventRow").await?;
// #[derive(Debug, Clone, clickhouse::Row, serde::Serialize, serde::Deserialize)]
// pub struct EventRow {
//     pub id: String,                                       // UUID
//     pub org: String,                                      // LowCardinality(String)
//     pub n: u64,
//     pub tags: Vec<String>,
//     pub attrs: std::collections::HashMap<String, String>,
// }
```

`ch_type_to_rust` / `rust_row_struct` are also exposed directly. Pair this with `check_drift` in CI to assert the generated Rust view stays ≡ the live (TS-owned) schema — so the Rust side can never silently diverge.

## Design

- **Safe by construction.** The type allowlist is unrepresentable-by-default; identifiers are validated + quoted; tables are bounded. The dangerous bits are impossible, not discouraged.
- **Rows are Serde-native.** Use `#[derive(clickhouse::Row, Deserialize)]` for reads — the kit doesn't reinvent row mapping.
- **Forward-only.** No auto-diff engine; schema changes are explicit migrations. The additive `ALTER` path for dynamic per-tenant tables is separate and bounded.
- **Tested against real ClickHouse.** The migration runner, drift gate, and DDL round-trip are verified via [testcontainers](https://crates.io/crates/testcontainers) in CI, not just string assertions.

## License

MIT © [SmooAI](https://smooai.com)
