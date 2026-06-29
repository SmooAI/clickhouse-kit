//! # smooai-clickhouse-kit (imports as `clickhouse_kit`)
//!
//! A safe-by-construction ClickHouse schema toolkit with two jobs:
//!
//! - **TS→Rust bridge** for developer-authored (static) tables: TypeScript owns
//!   the schema; [`introspect`] reads the live ClickHouse back into Rust and
//!   [`codegen`] emits `#[derive(Row)]` structs, with [`check_drift`](crate::drift)
//!   asserting the Rust view ≡ the live DB. Rows stay Serde-native.
//! - **Runtime toolkit** for user-defined / multi-tenant (dynamic) tables: an
//!   allowlisted type system, identifier safety, DDL generation, [`flexible_table`],
//!   forward-only migrations, and additive evolution — the safe-by-construction path
//!   for turning untrusted customer input into SQL (that guarantee only counts in
//!   the process holding the input, which is why this layer is canonical in Rust).
//!
//! See the repo `ROADMAP.md`.

pub mod client;
pub mod codegen;
pub mod drift;
pub mod evolve;
pub mod flatten;
pub mod flexible;
pub mod introspect;
pub mod migrate;
pub mod safety;
pub mod table;

pub use client::{ChError, ChExecutor};
pub use codegen::{
    ch_type_to_rust, emit_insert_schema, emit_row_interface, emit_select_schema, emit_ts_module,
    insert_schema_name, row_type_name, rust_row_struct, select_schema_name,
};
pub use drift::{check_drift, Drift, DriftResult};
pub use evolve::{alter_add_columns_sql, diff_columns, ColumnDiff, LiveColumn};
pub use flatten::{coerce_to_table, flatten_record, CoerceResult, FlattenOptions};
pub use flexible::{flexible_table, FlexibleConfig};
pub use introspect::{introspect_columns, introspect_row_struct};
pub use migrate::{run_migrations, split_sql_statements, MigrationRunResult};
pub use safety::{
    assert_column_count, assert_not_reserved, quote_identifier, validate_identifier,
    ColumnTypeSpec, DateTime64Spec, ScalarType, SchemaError, SchemaLimits, StringOnly,
    DEFAULT_RESERVED_COLUMNS,
};
pub use table::{to_create_table_sql, ColumnSpec, IndexSpec, TableSpec, TtlMove, TtlSpec};
