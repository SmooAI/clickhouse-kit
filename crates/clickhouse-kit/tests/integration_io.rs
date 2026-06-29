//! Integration tests for the I/O layer against a REAL ClickHouse (testcontainers).
//! Proves the migration runner applies SQL files and is idempotent, and that the
//! drift gate correctly reports a matching vs. divergent schema. Gated behind
//! `#[ignore]` so `cargo test` stays Docker-free; CI runs with `--ignored`.

use clickhouse::Client;
use clickhouse_kit::client::{ChError, ChExecutor, LiveColumn};
use clickhouse_kit::drift::{check_drift, Drift};
use clickhouse_kit::migrate::run_migrations;
use clickhouse_kit::{ColumnSpec, ColumnTypeSpec, ScalarType, TableSpec};
use std::future::Future;
use std::path::PathBuf;
use testcontainers_modules::{clickhouse::ClickHouse, testcontainers::runners::AsyncRunner};

/// Thin [`ChExecutor`] wrapper over the `clickhouse` crate's client — the BYO
/// client implementation a real consumer would write.
struct ClickHouseExec(Client);

// The `clickhouse` crate matches struct fields to result columns by name, so this
// row mirrors the single `filename` column that `fetch_strings` reads.
#[derive(clickhouse::Row, serde::Deserialize)]
struct StringRow {
    filename: String,
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct ColRow {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[allow(clippy::manual_async_fn)]
impl ChExecutor for ClickHouseExec {
    fn command(&self, sql: &str) -> impl Future<Output = Result<(), ChError>> + Send {
        async move {
            self.0
                .query(sql)
                .execute()
                .await
                .map_err(|e| ChError::Backend(e.to_string()))
        }
    }

    fn fetch_strings(
        &self,
        sql: &str,
    ) -> impl Future<Output = Result<Vec<String>, ChError>> + Send {
        async move {
            let rows = self
                .0
                .query(sql)
                .fetch_all::<StringRow>()
                .await
                .map_err(|e| ChError::Backend(e.to_string()))?;
            Ok(rows.into_iter().map(|r| r.filename).collect())
        }
    }

    fn fetch_columns(
        &self,
        table: &str,
    ) -> impl Future<Output = Result<Vec<LiveColumn>, ChError>> + Send {
        async move {
            let sql = format!(
                "SELECT name, type FROM system.columns \
                 WHERE database = currentDatabase() AND table = '{table}' ORDER BY position"
            );
            let rows = self
                .0
                .query(&sql)
                .fetch_all::<ColRow>()
                .await
                .map_err(|e| ChError::Backend(e.to_string()))?;
            Ok(rows
                .into_iter()
                .map(|r| LiveColumn {
                    name: r.name,
                    type_name: r.ty,
                })
                .collect())
        }
    }
}

fn col(name: &str, t: ScalarType) -> ColumnSpec {
    ColumnSpec {
        name: name.into(),
        type_spec: ColumnTypeSpec::Scalar(t),
        default: None,
    }
}

/// The `TableSpec` matching the migration's `CREATE TABLE events`.
fn events_spec() -> TableSpec {
    TableSpec {
        name: "events".into(),
        columns: vec![
            col("id", ScalarType::Uuid),
            col("ts", ScalarType::DateTime64),
            col("name", ScalarType::String),
        ],
        engine: "MergeTree()".into(),
        order_by: vec!["id".into()],
        partition_by: None,
        ttl: None,
        indexes: vec![],
        settings: vec![],
    }
}

/// Write the migration files into a fresh temp directory and return its path.
fn write_migrations() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("ch_mig_test_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("001_create_events.sql"),
        "-- create the events table\n\
         CREATE TABLE IF NOT EXISTS events (\n\
         \x20   id UUID,\n\
         \x20   ts DateTime64(3),\n\
         \x20   name String\n\
         ) ENGINE = MergeTree ORDER BY id;\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("002_seed_event.sql"),
        "INSERT INTO events VALUES (generateUUIDv4(), now64(3), 'hello');\n",
    )
    .unwrap();

    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker (ClickHouse testcontainer)"]
async fn migrations_apply_idempotently_and_drift_reports_correctly() {
    let node = ClickHouse::default()
        .start()
        .await
        .expect("start clickhouse container");
    let port = node.get_host_port_ipv4(8123).await.expect("http port");
    let client = Client::default().with_url(format!("http://127.0.0.1:{port}"));
    let exec = ClickHouseExec(client);

    let dir = write_migrations();

    // 1. First pass: both files applied, nothing skipped.
    let first = run_migrations(&exec, &dir)
        .await
        .expect("first migration run");
    assert_eq!(
        first.discovered,
        vec![
            "001_create_events.sql".to_string(),
            "002_seed_event.sql".to_string()
        ]
    );
    assert_eq!(first.applied, first.discovered);
    assert!(first.skipped.is_empty(), "first pass should skip nothing");

    // 2. Second pass: idempotent — everything skipped, nothing re-applied.
    let second = run_migrations(&exec, &dir)
        .await
        .expect("second migration run");
    assert!(
        second.applied.is_empty(),
        "second pass must apply nothing, got {:?}",
        second.applied
    );
    assert_eq!(second.skipped, second.discovered);

    // 3. Drift against the matching spec → clean.
    let clean = check_drift(&exec, &[events_spec()])
        .await
        .expect("drift check");
    assert!(clean.is_clean(), "expected no drift, got {:?}", clean.drift);

    // 4. Drift against a spec with an extra column → MissingColumn reported.
    let mut with_extra = events_spec();
    with_extra.columns.push(col("value", ScalarType::Float64));
    let drifted = check_drift(&exec, &[with_extra])
        .await
        .expect("drift check w/ extra column");
    assert!(
        drifted.drift.contains(&Drift::MissingColumn {
            table: "events".into(),
            column: "value".into(),
            expected_type: "Float64".into(),
        }),
        "expected MissingColumn drift for `value`, got {:?}",
        drifted.drift
    );

    std::fs::remove_dir_all(&dir).ok();
}
