//! TS→Rust bridge integration: introspect a real ClickHouse table and generate its
//! Rust row struct. Proves the live → Rust codegen path end-to-end. Gated behind
//! `#[ignore]` (Docker); CI runs it.

use clickhouse::Client;
use clickhouse_kit::{introspect_row_struct, ChError, ChExecutor, LiveColumn};
use std::future::Future;
use testcontainers_modules::{clickhouse::ClickHouse, testcontainers::runners::AsyncRunner};

struct Exec(Client);

#[derive(clickhouse::Row, serde::Deserialize)]
struct ColRow {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct StrRow {
    v: String,
}

#[allow(clippy::manual_async_fn)]
impl ChExecutor for Exec {
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
                .fetch_all::<StrRow>()
                .await
                .map_err(|e| ChError::Backend(e.to_string()))?;
            Ok(rows.into_iter().map(|r| r.v).collect())
        }
    }

    fn fetch_columns(
        &self,
        table: &str,
    ) -> impl Future<Output = Result<Vec<LiveColumn>, ChError>> + Send {
        async move {
            let q = format!("SELECT name, type FROM system.columns WHERE database = currentDatabase() AND table = '{table}' ORDER BY position");
            let rows = self
                .0
                .query(&q)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker (ClickHouse testcontainer)"]
async fn introspect_then_codegen_row_struct() {
    let node = ClickHouse::default()
        .start()
        .await
        .expect("start clickhouse");
    let port = node.get_host_port_ipv4(8123).await.expect("http port");
    let exec = Exec(Client::default().with_url(format!("http://127.0.0.1:{port}")));

    exec.command(
        "CREATE TABLE events (id UUID, org LowCardinality(String), n UInt64, ratio Float64, tags Array(String), attrs Map(String, String)) ENGINE = MergeTree() ORDER BY (id)",
    )
    .await
    .expect("create table");

    // Live ClickHouse table -> generated Rust row struct.
    let src = introspect_row_struct(&exec, "events", "EventRow")
        .await
        .expect("introspect + codegen");

    assert!(
        src.contains(
            "#[derive(Debug, Clone, clickhouse::Row, serde::Serialize, serde::Deserialize)]"
        ),
        "{src}"
    );
    assert!(src.contains("pub struct EventRow {"), "{src}");
    assert!(src.contains("pub id: String,"), "{src}"); // UUID -> String
    assert!(src.contains("pub org: String,"), "{src}"); // LowCardinality(String) -> String
    assert!(src.contains("pub n: u64,"), "{src}");
    assert!(src.contains("pub ratio: f64,"), "{src}");
    assert!(src.contains("pub tags: Vec<String>,"), "{src}");
    assert!(
        src.contains("pub attrs: std::collections::HashMap<String, String>,"),
        "{src}"
    );
}
