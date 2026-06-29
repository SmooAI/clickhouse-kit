//! Runtime table construction from untrusted specs → `CREATE TABLE` DDL.
//! Every identifier is validated and every type goes through the allowlist
//! ([`ColumnTypeSpec`]), so a spec built from customer input cannot inject SQL.

use crate::safety::{
    assert_column_count, validate_identifier, ColumnTypeSpec, SchemaError, SchemaLimits,
};
use std::collections::HashSet;

/// A single column in a runtime-built table.
#[derive(Debug, Clone)]
pub struct ColumnSpec {
    pub name: String,
    pub type_spec: ColumnTypeSpec,
    /// Optional ClickHouse DEFAULT expression (e.g. `now()`).
    pub default: Option<String>,
}

/// A secondary data-skipping index.
///
/// **Safety posture:** `name` is identifier-validated. `expression` and `type_def`
/// are **app-controlled raw SQL** (like [`TableSpec::engine`]) — they are emitted
/// verbatim, so never build them from untrusted input.
#[derive(Debug, Clone)]
pub struct IndexSpec {
    pub name: String,
    /// Raw, app-controlled index expression, e.g. `"trace_id"` or a real expression.
    pub expression: String,
    /// Raw, app-controlled index type, e.g. `"bloom_filter(0.01)"` or
    /// `"tokenbf_v1(8192, 3, 0)"`.
    pub type_def: String,
    pub granularity: u32,
}

/// A move-to-volume TTL tier.
///
/// **Safety posture:** both fields are app-controlled raw fragments emitted verbatim.
#[derive(Debug, Clone)]
pub struct TtlMove {
    /// Raw INTERVAL fragment, e.g. `"14 DAY"`.
    pub interval: String,
    /// Volume name, e.g. `"cold"`.
    pub volume: String,
}

/// Table TTL policy.
///
/// **Safety posture:** `column` is identifier-validated **and** must be a real column
/// in the table. `interval`/`volume`/`delete_after` are app-controlled raw fragments
/// emitted verbatim — never build them from untrusted input.
#[derive(Debug, Clone)]
pub struct TtlSpec {
    pub column: String,
    pub move_to_volume_after: Option<TtlMove>,
    /// Raw INTERVAL fragment for the DELETE tier, e.g. `"180 DAY"`.
    pub delete_after: Option<String>,
}

/// A table built from a runtime spec. `engine` is app-controlled (not user input);
/// `order_by` entries are validated as column identifiers.
///
/// **Safety posture for the production-DDL knobs** (`partition_by`, `ttl`, `indexes`,
/// `settings`): these are **app-controlled raw fragments** emitted verbatim, with the
/// sole exception that identifiers (`ttl.column`, `indexes[].name`) are validated and
/// `ttl.column` must be a real column. Never build the raw fragments from untrusted
/// input.
#[derive(Debug, Clone)]
pub struct TableSpec {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    pub engine: String,
    pub order_by: Vec<String>,
    /// App-controlled raw `PARTITION BY` expression, e.g.
    /// `"(organization_id, toDate(started_at))"`.
    pub partition_by: Option<String>,
    /// Optional table TTL policy.
    pub ttl: Option<TtlSpec>,
    /// Secondary data-skipping indexes rendered inside the column parens.
    pub indexes: Vec<IndexSpec>,
    /// App-controlled `SETTINGS` pairs (key, raw-value RHS), e.g.
    /// `("storage_policy", "'hot_cold'")`, `("index_granularity", "8192")`.
    pub settings: Vec<(String, String)>,
}

/// Render the `CREATE TABLE IF NOT EXISTS` DDL for a runtime spec, enforcing
/// identifier safety, the type allowlist, column bounds, and no duplicate columns.
pub fn to_create_table_sql(
    table: &TableSpec,
    limits: &SchemaLimits,
) -> Result<String, SchemaError> {
    validate_identifier(&table.name, "table", limits)?;
    assert_column_count(table.columns.len(), limits)?;

    let mut seen = HashSet::new();
    let mut col_lines = Vec::with_capacity(table.columns.len());
    for c in &table.columns {
        validate_identifier(&c.name, "column", limits)?;
        if !seen.insert(c.name.as_str()) {
            return Err(SchemaError::DuplicateColumn(c.name.clone()));
        }
        // Validate any untrusted type parameters (e.g. parametrised DateTime64
        // precision/timezone) before they reach the rendered SQL.
        c.type_spec.validate()?;
        let default = c
            .default
            .as_deref()
            .map(|d| format!(" DEFAULT {d}"))
            .unwrap_or_default();
        col_lines.push(format!(
            "    {} {}{}",
            c.name,
            c.type_spec.to_ch_type(),
            default
        ));
    }

    // ORDER BY entries must be real columns (validated identifiers) — no expressions
    // from untrusted input.
    let known: HashSet<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    for ob in &table.order_by {
        validate_identifier(ob, "column", limits)?;
        if !known.contains(ob.as_str()) {
            return Err(SchemaError::InvalidIdentifier {
                kind: "order_by column",
                name: ob.clone(),
            });
        }
    }

    // Secondary indexes render inside the column parens. `name` is identifier-validated;
    // `expression`/`type_def` are app-controlled raw SQL emitted verbatim.
    let mut paren_lines = col_lines;
    for idx in &table.indexes {
        validate_identifier(&idx.name, "index", limits)?;
        paren_lines.push(format!(
            "    INDEX {} {} TYPE {} GRANULARITY {}",
            idx.name, idx.expression, idx.type_def, idx.granularity
        ));
    }

    let mut sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n)\nENGINE = {}",
        table.name,
        paren_lines.join(",\n"),
        table.engine,
    );

    // PARTITION BY sits between ENGINE and ORDER BY.
    if let Some(partition_by) = &table.partition_by {
        sql.push_str(&format!("\nPARTITION BY {partition_by}"));
    }

    sql.push_str(&format!("\nORDER BY ({})", table.order_by.join(", ")));

    // TTL: the column must be a real, validated column. DateTime64 columns are wrapped
    // in `toDateTime(...)` for the TTL expression; everything else uses the bare column.
    if let Some(ttl) = &table.ttl {
        validate_identifier(&ttl.column, "column", limits)?;
        if !known.contains(ttl.column.as_str()) {
            return Err(SchemaError::InvalidIdentifier {
                kind: "ttl column",
                name: ttl.column.clone(),
            });
        }
        let type_spec = table
            .columns
            .iter()
            .find(|c| c.name == ttl.column)
            .map(|c| &c.type_spec);
        let base = match type_spec {
            Some(ts) if ts.is_datetime64() => format!("toDateTime({})", ttl.column),
            _ => ttl.column.clone(),
        };
        let mut parts = Vec::new();
        if let Some(mv) = &ttl.move_to_volume_after {
            parts.push(format!(
                "{base} + INTERVAL {} TO VOLUME '{}'",
                mv.interval, mv.volume
            ));
        }
        if let Some(after) = &ttl.delete_after {
            parts.push(format!("{base} + INTERVAL {after} DELETE"));
        }
        if !parts.is_empty() {
            sql.push_str(&format!("\nTTL {}", parts.join(", ")));
        }
    }

    // SETTINGS render last. Values are app-controlled raw RHS fragments.
    if !table.settings.is_empty() {
        let rendered: Vec<String> = table
            .settings
            .iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect();
        sql.push_str(&format!("\nSETTINGS {}", rendered.join(", ")));
    }

    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::ScalarType;

    fn col(name: &str, t: ColumnTypeSpec) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            type_spec: t,
            default: None,
        }
    }

    fn sample() -> TableSpec {
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
                        array: crate::safety::StringOnly::String,
                    },
                ),
            ],
            engine: "MergeTree()".into(),
            order_by: vec!["id".into()],
            partition_by: None,
            ttl: None,
            indexes: vec![],
            settings: vec![],
        }
    }

    #[test]
    fn renders_create_table() {
        let ddl = to_create_table_sql(&sample(), &SchemaLimits::default()).unwrap();
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS events ("));
        assert!(ddl.contains("id UUID"));
        assert!(ddl.contains("ts DateTime64(3)"));
        assert!(ddl.contains("tags Array(String)"));
        assert!(ddl.contains("ENGINE = MergeTree()"));
        assert!(ddl.contains("ORDER BY (id)"));
    }

    #[test]
    fn rejects_duplicate_and_bad_identifiers() {
        let mut t = sample();
        t.columns
            .push(col("id", ColumnTypeSpec::Scalar(ScalarType::String)));
        assert!(matches!(
            to_create_table_sql(&t, &SchemaLimits::default()),
            Err(SchemaError::DuplicateColumn(_))
        ));

        let mut t2 = sample();
        t2.columns[0].name = "id; DROP TABLE x".into();
        assert!(to_create_table_sql(&t2, &SchemaLimits::default()).is_err());
    }

    #[test]
    fn rejects_order_by_unknown_column() {
        let mut t = sample();
        t.order_by = vec!["nope".into()];
        assert!(to_create_table_sql(&t, &SchemaLimits::default()).is_err());
    }

    /// The live `observability_traces` table — a real production DDL with partitioning,
    /// two data-skipping indexes, a two-tier TTL, and settings.
    fn observability_traces() -> TableSpec {
        TableSpec {
            name: "observability_traces".into(),
            columns: vec![
                col("started_at", ColumnTypeSpec::Scalar(ScalarType::DateTime64)),
                col(
                    "organization_id",
                    ColumnTypeSpec::LowCardinality {
                        low_cardinality: Box::new(ColumnTypeSpec::Scalar(ScalarType::String)),
                    },
                ),
                col("trace_id", ColumnTypeSpec::Scalar(ScalarType::String)),
                col("name", ColumnTypeSpec::Scalar(ScalarType::String)),
                col(
                    "service_name",
                    ColumnTypeSpec::LowCardinality {
                        low_cardinality: Box::new(ColumnTypeSpec::Scalar(ScalarType::String)),
                    },
                ),
                col("has_error", ColumnTypeSpec::Scalar(ScalarType::UInt8)),
                col(
                    "attributes",
                    ColumnTypeSpec::Map {
                        map: (
                            crate::safety::StringOnly::String,
                            crate::safety::StringOnly::String,
                        ),
                    },
                ),
                ColumnSpec {
                    name: "ingested_at".into(),
                    type_spec: ColumnTypeSpec::Scalar(ScalarType::DateTime),
                    default: Some("now()".into()),
                },
            ],
            engine: "MergeTree()".into(),
            order_by: vec![
                "organization_id".into(),
                "service_name".into(),
                "started_at".into(),
                "trace_id".into(),
            ],
            partition_by: Some("(organization_id, toDate(started_at))".into()),
            ttl: Some(TtlSpec {
                column: "started_at".into(),
                move_to_volume_after: Some(TtlMove {
                    interval: "14 DAY".into(),
                    volume: "cold".into(),
                }),
                delete_after: Some("180 DAY".into()),
            }),
            indexes: vec![
                IndexSpec {
                    name: "idx_trace_id".into(),
                    expression: "trace_id".into(),
                    type_def: "bloom_filter(0.01)".into(),
                    granularity: 1,
                },
                IndexSpec {
                    name: "idx_name".into(),
                    expression: "name".into(),
                    type_def: "tokenbf_v1(8192, 3, 0)".into(),
                    granularity: 1,
                },
            ],
            settings: vec![
                ("storage_policy".into(), "'hot_cold'".into()),
                ("index_granularity".into(), "8192".into()),
            ],
        }
    }

    #[test]
    fn reproduces_observability_traces_production_ddl() {
        let ddl = to_create_table_sql(&observability_traces(), &SchemaLimits::default()).unwrap();

        // Partitioning between ENGINE and ORDER BY.
        assert!(
            ddl.contains("PARTITION BY (organization_id, toDate(started_at))"),
            "{ddl}"
        );
        // Both INDEX lines, verbatim, inside the column parens.
        assert!(
            ddl.contains("    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 1"),
            "{ddl}"
        );
        assert!(
            ddl.contains("    INDEX idx_name name TYPE tokenbf_v1(8192, 3, 0) GRANULARITY 1"),
            "{ddl}"
        );
        // TTL line, verbatim — started_at is DateTime64 → wrapped in toDateTime(...).
        assert!(
            ddl.contains("TTL toDateTime(started_at) + INTERVAL 14 DAY TO VOLUME 'cold', toDateTime(started_at) + INTERVAL 180 DAY DELETE"),
            "{ddl}"
        );
        // SETTINGS line, verbatim, last.
        assert!(
            ddl.contains("SETTINGS storage_policy = 'hot_cold', index_granularity = 8192"),
            "{ddl}"
        );

        // Clause ordering sanity: ENGINE < PARTITION BY < ORDER BY < TTL < SETTINGS.
        let pos = |needle: &str| ddl.find(needle).unwrap();
        assert!(pos("ENGINE = MergeTree()") < pos("PARTITION BY"));
        assert!(pos("PARTITION BY") < pos("ORDER BY ("));
        assert!(pos("ORDER BY (") < pos("TTL "));
        assert!(pos("TTL ") < pos("SETTINGS "));
    }

    #[test]
    fn ttl_on_plain_datetime_is_not_wrapped() {
        let mut t = sample();
        t.columns
            .push(col("created", ColumnTypeSpec::Scalar(ScalarType::DateTime)));
        t.ttl = Some(TtlSpec {
            column: "created".into(),
            move_to_volume_after: None,
            delete_after: Some("30 DAY".into()),
        });
        let ddl = to_create_table_sql(&t, &SchemaLimits::default()).unwrap();
        assert!(
            ddl.contains("TTL created + INTERVAL 30 DAY DELETE"),
            "{ddl}"
        );
        assert!(!ddl.contains("toDateTime(created)"), "{ddl}");
    }

    #[test]
    fn ttl_delete_only_renders_just_delete() {
        let mut t = sample();
        // `ts` is DateTime64 → wrapped.
        t.ttl = Some(TtlSpec {
            column: "ts".into(),
            move_to_volume_after: None,
            delete_after: Some("90 DAY".into()),
        });
        let ddl = to_create_table_sql(&t, &SchemaLimits::default()).unwrap();
        assert!(
            ddl.contains("TTL toDateTime(ts) + INTERVAL 90 DAY DELETE"),
            "{ddl}"
        );
        assert!(!ddl.contains("TO VOLUME"), "{ddl}");
    }

    #[test]
    fn ttl_unknown_column_is_rejected() {
        let mut t = sample();
        t.ttl = Some(TtlSpec {
            column: "nope".into(),
            move_to_volume_after: None,
            delete_after: Some("1 DAY".into()),
        });
        assert!(matches!(
            to_create_table_sql(&t, &SchemaLimits::default()),
            Err(SchemaError::InvalidIdentifier {
                kind: "ttl column",
                ..
            })
        ));
    }

    #[test]
    fn index_with_invalid_name_is_rejected() {
        let mut t = sample();
        t.indexes = vec![IndexSpec {
            name: "bad name".into(),
            expression: "name".into(),
            type_def: "bloom_filter(0.01)".into(),
            granularity: 1,
        }];
        assert!(matches!(
            to_create_table_sql(&t, &SchemaLimits::default()),
            Err(SchemaError::InvalidIdentifier { kind: "index", .. })
        ));
    }

    #[test]
    fn backward_compat_no_extra_clauses() {
        // With all the new knobs absent, the output is exactly the legacy shape:
        // no PARTITION BY / TTL / SETTINGS lines, no trailing INDEX lines.
        let ddl = to_create_table_sql(&sample(), &SchemaLimits::default()).unwrap();
        let expected = "CREATE TABLE IF NOT EXISTS events (\n    id UUID,\n    ts DateTime64(3),\n    name String,\n    value Float64,\n    tags Array(String)\n)\nENGINE = MergeTree()\nORDER BY (id)";
        assert_eq!(ddl, expected);
    }

    #[test]
    fn parametrised_datetime64_column_renders_with_timezone() {
        let mut t = sample();
        let dt: ColumnTypeSpec =
            serde_json::from_str(r#"{"datetime64":{"precision":3,"timezone":"UTC"}}"#).unwrap();
        t.columns.push(col("occurred_at", dt));
        let ddl = to_create_table_sql(&t, &SchemaLimits::default()).unwrap();
        assert!(ddl.contains("occurred_at DateTime64(3, 'UTC')"), "{ddl}");
    }

    #[test]
    fn parametrised_datetime64_bad_params_rejected_at_ddl_boundary() {
        // Bad timezone is caught in the per-column loop, before reaching SQL.
        let mut t = sample();
        let bad_tz: ColumnTypeSpec =
            serde_json::from_str(r#"{"datetime64":{"precision":3,"timezone":"UTC'; DROP"}}"#)
                .unwrap();
        t.columns.push(col("occurred_at", bad_tz));
        assert!(matches!(
            to_create_table_sql(&t, &SchemaLimits::default()),
            Err(SchemaError::InvalidIdentifier {
                kind: "timezone",
                ..
            })
        ));

        // Out-of-range precision is also caught.
        let mut t2 = sample();
        let bad_p: ColumnTypeSpec =
            serde_json::from_str(r#"{"datetime64":{"precision":12}}"#).unwrap();
        t2.columns.push(col("occurred_at", bad_p));
        assert!(matches!(
            to_create_table_sql(&t2, &SchemaLimits::default()),
            Err(SchemaError::InvalidDateTime64Precision { precision: 12 })
        ));
    }

    #[test]
    fn ttl_wraps_parametrised_datetime64_column() {
        let mut t = sample();
        let dt: ColumnTypeSpec =
            serde_json::from_str(r#"{"datetime64":{"precision":3,"timezone":"UTC"}}"#).unwrap();
        t.columns.push(col("occurred_at", dt));
        t.ttl = Some(TtlSpec {
            column: "occurred_at".into(),
            move_to_volume_after: None,
            delete_after: Some("30 DAY".into()),
        });
        let ddl = to_create_table_sql(&t, &SchemaLimits::default()).unwrap();
        // The parametrised DateTime64 column is still wrapped in toDateTime(...).
        assert!(
            ddl.contains("TTL toDateTime(occurred_at) + INTERVAL 30 DAY DELETE"),
            "{ddl}"
        );
    }
}
