//! # clickhouse-kit
//!
//! Safe-by-construction schema toolkit for ClickHouse — the Rust-canonical port of
//! [`@smooai/clickhouse-kit`](https://github.com/SmooAI/clickhouse-kit). Rows are
//! Serde-native (use the `clickhouse` crate's `#[derive(Row)]`); this crate adds
//! what Serde doesn't: an allowlisted type system for **user-defined / multi-tenant**
//! schemas, identifier safety, DDL generation, and additive evolution.
//!
//! The safety layer lives here because that's where untrusted customer input is
//! turned into SQL — safe-by-construction only counts in the process holding the
//! input. See the repo `ROADMAP.md`.

pub mod client;
pub mod drift;
pub mod evolve;
pub mod flatten;
pub mod flexible;
pub mod migrate;
pub mod safety;
pub mod table;

pub use client::{ChError, ChExecutor};
pub use drift::{check_drift, Drift, DriftResult};
pub use evolve::{alter_add_columns_sql, diff_columns, ColumnDiff, LiveColumn};
pub use flatten::{coerce_to_table, flatten_record, CoerceResult, FlattenOptions};
pub use flexible::{flexible_table, FlexibleConfig};
pub use migrate::{run_migrations, split_sql_statements, MigrationRunResult};
pub use safety::{
    assert_column_count, assert_not_reserved, quote_identifier, validate_identifier,
    ColumnTypeSpec, ScalarType, SchemaError, SchemaLimits, StringOnly, DEFAULT_RESERVED_COLUMNS,
};
pub use table::{to_create_table_sql, ColumnSpec, TableSpec};
