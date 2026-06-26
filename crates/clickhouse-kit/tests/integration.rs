//! Integration tests against a REAL ClickHouse (via testcontainers). Proves the
//! generated DDL is valid, applies cleanly, produces the exact schema we modeled,
//! and yields a usable table. Gated behind `#[ignore]` so `cargo test` stays
//! Docker-free; CI runs them with `--include-ignored`.

use clickhouse::Client;
use clickhouse_kit::{
    to_create_table_sql, ColumnSpec, ColumnTypeSpec, ScalarType, SchemaLimits, StringOnly,
    TableSpec,
};
use testcontainers_modules::{clickhouse::ClickHouse, testcontainers::runners::AsyncRunner};

fn col(name: &str, t: ColumnTypeSpec) -> ColumnSpec {
    ColumnSpec {
        name: name.into(),
        type_spec: t,
        default: None,
    }
}

fn events_table() -> TableSpec {
    TableSpec {
        name: "events".into(),
        columns: vec![
            col("id", ColumnTypeSpec::Scalar(ScalarType::Uuid)),
            col("ts", ColumnTypeSpec::Scalar(ScalarType::DateTime64)),
            col("name", ColumnTypeSpec::Scalar(ScalarType::String)),
            col("value", ColumnTypeSpec::Scalar(ScalarType::Float64)),
            col(
                "tags",
                ColumnTypeSpec::Array {
                    array: StringOnly::String,
                },
            ),
            col(
                "attrs",
                ColumnTypeSpec::Map {
                    map: (StringOnly::String, StringOnly::String),
                },
            ),
        ],
        engine: "MergeTree()".into(),
        order_by: vec!["id".into()],
    }
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct ColInfo {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct NameRow {
    name: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker (ClickHouse testcontainer)"]
async fn generated_ddl_applies_and_roundtrips() {
    let node = ClickHouse::default()
        .start()
        .await
        .expect("start clickhouse container");
    let port = node.get_host_port_ipv4(8123).await.expect("http port");
    let client = Client::default().with_url(format!("http://127.0.0.1:{port}"));

    // 1. Generate DDL from a runtime (untrusted-shaped) spec and apply it to REAL ClickHouse.
    let ddl = to_create_table_sql(&events_table(), &SchemaLimits::default()).unwrap();
    client
        .query(&ddl)
        .execute()
        .await
        .expect("CREATE TABLE applies cleanly");

    // 2. Introspect system.columns — the live schema must equal what we generated.
    let cols = client
        .query("SELECT name, type FROM system.columns WHERE database = currentDatabase() AND table = 'events' ORDER BY position")
        .fetch_all::<ColInfo>()
        .await
        .expect("introspect columns");
    let actual: Vec<(String, String)> = cols.into_iter().map(|c| (c.name, c.ty)).collect();
    assert_eq!(
        actual,
        vec![
            ("id".to_string(), "UUID".to_string()),
            ("ts".to_string(), "DateTime64(3)".to_string()),
            ("name".to_string(), "String".to_string()),
            ("value".to_string(), "Float64".to_string()),
            ("tags".to_string(), "Array(String)".to_string()),
            ("attrs".to_string(), "Map(String, String)".to_string()),
        ]
    );

    // 3. Insert + read back — proves the generated table is actually usable.
    client
        .query("INSERT INTO events VALUES (generateUUIDv4(), now64(3), 'hello', 1.5, ['a','b'], map('k','v'))")
        .execute()
        .await
        .expect("insert");
    let names = client
        .query("SELECT name FROM events")
        .fetch_all::<NameRow>()
        .await
        .expect("select");
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name, "hello");
}
