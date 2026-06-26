//! The bring-your-own-client seam. `clickhouse-kit` never depends on a concrete
//! ClickHouse driver — the I/O layer (migration runner, drift gate) is written
//! against the small [`ChExecutor`] trait, and the caller implements it over
//! whatever client they already have (the `clickhouse` crate, an HTTP shim, a
//! test double). This keeps the crate driver-agnostic and dependency-light.

use std::future::Future;

/// Errors surfaced by the I/O layer — either the backing client failed, or we
/// hit a local filesystem error while reading migration files.
#[derive(Debug, thiserror::Error)]
pub enum ChError {
    /// The underlying ClickHouse client returned an error. The caller's
    /// [`ChExecutor`] implementation maps its driver error into this string.
    #[error("clickhouse backend error: {0}")]
    Backend(String),
    /// A local filesystem error (e.g. reading the migrations directory).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A single live column as introspected from `system.columns` — name + the
/// ClickHouse type string. The canonical definition lives in [`crate::evolve`];
/// it's re-exported here so the I/O layer and the drift/evolve modules all share
/// exactly one `LiveColumn` type.
pub use crate::evolve::LiveColumn;

/// The minimal async execution surface the I/O layer needs from a ClickHouse
/// client. Implement it over your driver of choice.
///
/// Methods return `impl Future + Send` rather than using `async fn` directly so
/// the futures are guaranteed `Send` (spawn-friendly) regardless of toolchain
/// object-safety quirks; an impl may still write `async fn`. (The explicit
/// `+ Send` is the whole point, so `manual_async_fn` is intentionally allowed.)
#[allow(clippy::manual_async_fn)]
pub trait ChExecutor {
    /// Run a single statement that returns no rows (DDL, INSERT, …).
    fn command(&self, sql: &str) -> impl Future<Output = Result<(), ChError>> + Send;

    /// Run a query whose result is a single `String` column, returning one entry
    /// per row (used for applied-migration filenames).
    fn fetch_strings(&self, sql: &str)
        -> impl Future<Output = Result<Vec<String>, ChError>> + Send;

    /// Introspect the live columns (name + type) of `table` from
    /// `system.columns`, in declaration order.
    fn fetch_columns(
        &self,
        table: &str,
    ) -> impl Future<Output = Result<Vec<LiveColumn>, ChError>> + Send;
}
