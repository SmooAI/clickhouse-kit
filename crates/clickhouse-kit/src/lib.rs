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

pub mod safety;
pub mod table;

pub use safety::{
    assert_column_count, assert_not_reserved, quote_identifier, validate_identifier,
    ColumnTypeSpec, ScalarType, SchemaError, SchemaLimits, StringOnly, DEFAULT_RESERVED_COLUMNS,
};
pub use table::{to_create_table_sql, ColumnSpec, TableSpec};
