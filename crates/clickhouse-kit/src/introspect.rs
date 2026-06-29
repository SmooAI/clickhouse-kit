//! The live→Rust half of the TS→Rust bridge. TypeScript authors the (static)
//! schema and ClickHouse holds it; this reads it back into Rust — columns for
//! `check_drift`, and a generated row struct via `codegen` — so the Rust side is a
//! faithful, drift-checked view of the TS-owned schema and can never silently
//! diverge.

use crate::client::{ChError, ChExecutor};
use crate::codegen::rust_row_struct;
use crate::evolve::LiveColumn;

/// Introspect a table's live columns (name + ClickHouse type) from `system.columns`.
pub async fn introspect_columns(
    exec: &impl ChExecutor,
    table: &str,
) -> Result<Vec<LiveColumn>, ChError> {
    exec.fetch_columns(table).await
}

/// Introspect a live table and generate its Rust row struct source — the bridge
/// one-liner (a TS-authored ClickHouse table → a Rust `#[derive(Row)]` struct).
pub async fn introspect_row_struct(
    exec: &impl ChExecutor,
    table: &str,
    struct_name: &str,
) -> Result<String, ChError> {
    let cols = introspect_columns(exec, table).await?;
    let pairs: Vec<(String, String)> = cols.into_iter().map(|c| (c.name, c.type_name)).collect();
    Ok(rust_row_struct(struct_name, &pairs))
}
