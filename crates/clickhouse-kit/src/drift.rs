//! Drift gate: compare a set of expected [`TableSpec`]s against the live schema
//! introspected from `system.columns`. Read-only — it reports divergence, it
//! never mutates. Intended to run in CI to catch a deployed schema that no
//! longer matches the code's model.

use crate::client::{ChError, ChExecutor};
use crate::table::TableSpec;
use std::collections::HashMap;

/// A single schema divergence between the expected spec and the live database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// The table is expected but does not exist (no live columns).
    MissingTable { table: String },
    /// A column is in the spec but missing from the live table.
    MissingColumn {
        table: String,
        column: String,
        expected_type: String,
    },
    /// A column exists live but is not in the spec.
    ExtraColumn {
        table: String,
        column: String,
        actual_type: String,
    },
    /// A column exists on both sides with a different type.
    TypeMismatch {
        table: String,
        column: String,
        expected_type: String,
        actual_type: String,
    },
}

/// Result of a [`check_drift`] pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftResult {
    /// Names of every table that was checked.
    pub checked: Vec<String>,
    /// All divergences found (empty == schema matches).
    pub drift: Vec<Drift>,
}

impl DriftResult {
    /// Whether the live schema matches every expected spec.
    pub fn is_clean(&self) -> bool {
        self.drift.is_empty()
    }
}

/// Normalize a ClickHouse type string for comparison by stripping all
/// whitespace, so `Map(String, String)` and `Map(String,String)` match.
fn normalize_type(ch_type: &str) -> String {
    ch_type.chars().filter(|c| !c.is_whitespace()).collect()
}

/// For each expected table, introspect its live columns and report any drift.
pub async fn check_drift(
    exec: &impl ChExecutor,
    tables: &[TableSpec],
) -> Result<DriftResult, ChError> {
    let mut checked = Vec::with_capacity(tables.len());
    let mut drift = Vec::new();

    for table in tables {
        checked.push(table.name.clone());

        let live = exec.fetch_columns(&table.name).await?;
        if live.is_empty() {
            drift.push(Drift::MissingTable {
                table: table.name.clone(),
            });
            continue;
        }

        let live_by_name: HashMap<&str, String> = live
            .iter()
            .map(|c| (c.name.as_str(), normalize_type(&c.type_name)))
            .collect();
        let expected_names: HashMap<&str, ()> = table
            .columns
            .iter()
            .map(|c| (c.name.as_str(), ()))
            .collect();

        // Expected columns: present-and-matching, present-but-wrong-type, or missing.
        for col in &table.columns {
            let expected_type = col.type_spec.to_ch_type();
            match live_by_name.get(col.name.as_str()) {
                None => drift.push(Drift::MissingColumn {
                    table: table.name.clone(),
                    column: col.name.clone(),
                    expected_type,
                }),
                Some(actual) if *actual != normalize_type(&expected_type) => {
                    drift.push(Drift::TypeMismatch {
                        table: table.name.clone(),
                        column: col.name.clone(),
                        expected_type,
                        actual_type: actual.clone(),
                    });
                }
                Some(_) => {}
            }
        }

        // Live columns not in the spec.
        for col in &live {
            if !expected_names.contains_key(col.name.as_str()) {
                drift.push(Drift::ExtraColumn {
                    table: table.name.clone(),
                    column: col.name.clone(),
                    actual_type: col.type_name.clone(),
                });
            }
        }
    }

    Ok(DriftResult { checked, drift })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::LiveColumn;
    use crate::safety::ScalarType;
    use crate::table::{ColumnSpec, TableSpec};
    use crate::ColumnTypeSpec;
    use std::future::Future;

    fn col(name: &str, t: ScalarType) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            type_spec: ColumnTypeSpec::Scalar(t),
            default: None,
        }
    }

    fn spec() -> TableSpec {
        TableSpec {
            name: "events".into(),
            columns: vec![col("id", ScalarType::Uuid), col("name", ScalarType::String)],
            engine: "MergeTree()".into(),
            order_by: vec!["id".into()],
            partition_by: None,
            ttl: None,
            indexes: vec![],
            settings: vec![],
        }
    }

    /// A canned executor: drift only needs `fetch_columns`, the others are unused.
    struct FakeExec(Vec<LiveColumn>);

    fn lc(name: &str, ty: &str) -> LiveColumn {
        LiveColumn {
            name: name.into(),
            type_name: ty.into(),
        }
    }

    #[allow(clippy::manual_async_fn)]
    impl ChExecutor for FakeExec {
        fn command(&self, _sql: &str) -> impl Future<Output = Result<(), ChError>> + Send {
            async { Ok(()) }
        }
        fn fetch_strings(
            &self,
            _sql: &str,
        ) -> impl Future<Output = Result<Vec<String>, ChError>> + Send {
            async { Ok(vec![]) }
        }
        fn fetch_columns(
            &self,
            _table: &str,
        ) -> impl Future<Output = Result<Vec<LiveColumn>, ChError>> + Send {
            let cols = self.0.clone();
            async move { Ok(cols) }
        }
    }

    #[tokio::test]
    async fn no_drift_when_schema_matches() {
        let exec = FakeExec(vec![lc("id", "UUID"), lc("name", "String")]);
        let result = check_drift(&exec, &[spec()]).await.unwrap();
        assert_eq!(result.checked, vec!["events".to_string()]);
        assert!(
            result.is_clean(),
            "expected no drift, got {:?}",
            result.drift
        );
    }

    #[tokio::test]
    async fn reports_missing_table() {
        let exec = FakeExec(vec![]);
        let result = check_drift(&exec, &[spec()]).await.unwrap();
        assert_eq!(
            result.drift,
            vec![Drift::MissingTable {
                table: "events".into()
            }]
        );
    }

    #[tokio::test]
    async fn reports_missing_extra_and_mismatch() {
        // `name` missing, `id` wrong type, `extra` not in spec.
        let exec = FakeExec(vec![lc("id", "String"), lc("extra", "Int32")]);
        let result = check_drift(&exec, &[spec()]).await.unwrap();
        assert!(result.drift.contains(&Drift::TypeMismatch {
            table: "events".into(),
            column: "id".into(),
            expected_type: "UUID".into(),
            actual_type: "String".into(),
        }));
        assert!(result.drift.contains(&Drift::MissingColumn {
            table: "events".into(),
            column: "name".into(),
            expected_type: "String".into(),
        }));
        assert!(result.drift.contains(&Drift::ExtraColumn {
            table: "events".into(),
            column: "extra".into(),
            actual_type: "Int32".into(),
        }));
    }

    #[tokio::test]
    async fn whitespace_in_types_is_normalized() {
        let mut s = spec();
        s.columns.push(ColumnSpec {
            name: "attrs".into(),
            type_spec: ColumnTypeSpec::Map {
                map: (
                    crate::safety::StringOnly::String,
                    crate::safety::StringOnly::String,
                ),
            },
            default: None,
        });
        // Live type has no space after the comma; spec renders with a space.
        let exec = FakeExec(vec![
            lc("id", "UUID"),
            lc("name", "String"),
            lc("attrs", "Map(String,String)"),
        ]);
        let result = check_drift(&exec, &[s]).await.unwrap();
        assert!(result.is_clean(), "drift: {:?}", result.drift);
    }
}
