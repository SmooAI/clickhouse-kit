//! The flexible/hybrid multi-tenant table — the most-reused shape.
//!
//! A flexible table pins a set of app-controlled **mandatory** columns and any
//! **promoted** columns (attributes lifted out of the catch-all for indexing /
//! ordering), then always appends two reserved columns: `attrs` — a
//! `Map(String, String)` catch-all for the long tail of per-tenant attributes —
//! and `raw` — the untouched source payload as a `String`. Every caller-supplied
//! name is validated and checked against the reserved set, so a config built from
//! untrusted input cannot inject SQL or shadow the reserved columns.

use crate::safety::{
    assert_not_reserved, validate_identifier, ColumnTypeSpec, ScalarType, SchemaError,
    SchemaLimits, StringOnly, DEFAULT_RESERVED_COLUMNS,
};
use crate::table::{ColumnSpec, IndexSpec, TableSpec, TtlSpec};

/// Configuration for a flexible/hybrid table.
///
/// `mandatory` + `promoted` are the explicit, queryable columns; `attrs` (catch-all)
/// and `raw` (source payload) are appended automatically. `reserved` overrides the
/// default reserved set ([`DEFAULT_RESERVED_COLUMNS`]) when supplied.
#[derive(Debug, Clone)]
pub struct FlexibleConfig {
    pub mandatory: Vec<ColumnSpec>,
    pub promoted: Vec<ColumnSpec>,
    pub engine: String,
    pub order_by: Vec<String>,
    pub reserved: Option<Vec<String>>,
    /// App-controlled raw `PARTITION BY` expression — copied verbatim into the
    /// produced [`TableSpec`]. See [`TableSpec`] for the safety posture.
    pub partition_by: Option<String>,
    /// Optional table TTL policy — copied verbatim into the produced [`TableSpec`].
    pub ttl: Option<TtlSpec>,
    /// Secondary data-skipping indexes — copied verbatim into the produced [`TableSpec`].
    pub indexes: Vec<IndexSpec>,
    /// App-controlled `SETTINGS` pairs — copied verbatim into the produced [`TableSpec`].
    pub settings: Vec<(String, String)>,
}

/// Build a [`TableSpec`] for the flexible/hybrid table shape: the mandatory + promoted
/// columns followed by the reserved `attrs Map(String, String)` and `raw String`
/// columns. Validates the table name and every caller column name, and rejects any
/// caller column that collides with the reserved set. Render with
/// [`crate::table::to_create_table_sql`].
pub fn flexible_table(
    name: &str,
    config: FlexibleConfig,
    limits: &SchemaLimits,
) -> Result<TableSpec, SchemaError> {
    validate_identifier(name, "table", limits)?;

    // The reserved set the caller's columns must not collide with.
    let reserved_owned: Option<Vec<String>> = config.reserved.clone();
    let reserved: Vec<&str> = match &reserved_owned {
        Some(r) => r.iter().map(String::as_str).collect(),
        None => DEFAULT_RESERVED_COLUMNS.to_vec(),
    };

    for c in config.mandatory.iter().chain(config.promoted.iter()) {
        validate_identifier(&c.name, "column", limits)?;
        assert_not_reserved(&c.name, &reserved)?;
    }

    let mut columns: Vec<ColumnSpec> =
        Vec::with_capacity(config.mandatory.len() + config.promoted.len() + 2);
    columns.extend(config.mandatory);
    columns.extend(config.promoted);
    columns.push(ColumnSpec {
        name: "attrs".into(),
        type_spec: ColumnTypeSpec::Map {
            map: (StringOnly::String, StringOnly::String),
        },
        default: None,
    });
    columns.push(ColumnSpec {
        name: "raw".into(),
        type_spec: ColumnTypeSpec::Scalar(ScalarType::String),
        default: None,
    });

    Ok(TableSpec {
        name: name.to_string(),
        columns,
        engine: config.engine,
        order_by: config.order_by,
        partition_by: config.partition_by,
        ttl: config.ttl,
        indexes: config.indexes,
        settings: config.settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::to_create_table_sql;

    fn col(name: &str, t: ColumnTypeSpec) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            type_spec: t,
            default: None,
        }
    }

    fn config() -> FlexibleConfig {
        FlexibleConfig {
            mandatory: vec![
                col("org_id", ColumnTypeSpec::Scalar(ScalarType::String)),
                col("ts", ColumnTypeSpec::Scalar(ScalarType::DateTime64)),
            ],
            promoted: vec![col("status", ColumnTypeSpec::Scalar(ScalarType::String))],
            engine: "MergeTree()".into(),
            order_by: vec!["org_id".into(), "ts".into()],
            reserved: None,
            partition_by: None,
            ttl: None,
            indexes: vec![],
            settings: vec![],
        }
    }

    #[test]
    fn renders_flexible_table() {
        let spec = flexible_table("events", config(), &SchemaLimits::default()).unwrap();
        let ddl = to_create_table_sql(&spec, &SchemaLimits::default()).unwrap();
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS events ("));
        assert!(ddl.contains("org_id String"));
        assert!(ddl.contains("ts DateTime64(3)"));
        assert!(ddl.contains("status String"));
        assert!(ddl.contains("attrs Map(String, String)"));
        assert!(ddl.contains("raw String"));
        assert!(ddl.contains("ENGINE = MergeTree()"));
        assert!(ddl.contains("ORDER BY (org_id, ts)"));
    }

    #[test]
    fn carries_partition_ttl_indexes_settings_into_ddl() {
        use crate::table::{IndexSpec, TtlMove, TtlSpec};

        let mut cfg = config();
        cfg.partition_by = Some("(org_id, toDate(ts))".into());
        cfg.ttl = Some(TtlSpec {
            column: "ts".into(),
            move_to_volume_after: Some(TtlMove {
                interval: "7 DAY".into(),
                volume: "cold".into(),
            }),
            delete_after: Some("90 DAY".into()),
        });
        cfg.indexes = vec![IndexSpec {
            name: "idx_status".into(),
            expression: "status".into(),
            type_def: "bloom_filter(0.01)".into(),
            granularity: 1,
        }];
        cfg.settings = vec![("index_granularity".into(), "8192".into())];

        let spec = flexible_table("events", cfg, &SchemaLimits::default()).unwrap();
        let ddl = to_create_table_sql(&spec, &SchemaLimits::default()).unwrap();

        assert!(ddl.contains("PARTITION BY (org_id, toDate(ts))"), "{ddl}");
        assert!(
            ddl.contains("    INDEX idx_status status TYPE bloom_filter(0.01) GRANULARITY 1"),
            "{ddl}"
        );
        assert!(
            ddl.contains("TTL toDateTime(ts) + INTERVAL 7 DAY TO VOLUME 'cold', toDateTime(ts) + INTERVAL 90 DAY DELETE"),
            "{ddl}"
        );
        assert!(ddl.contains("SETTINGS index_granularity = 8192"), "{ddl}");
    }

    #[test]
    fn rejects_promoted_column_colliding_with_reserved() {
        let mut cfg = config();
        cfg.promoted
            .push(col("attrs", ColumnTypeSpec::Scalar(ScalarType::String)));
        assert!(matches!(
            flexible_table("events", cfg, &SchemaLimits::default()),
            Err(SchemaError::ReservedColumn(_))
        ));
    }

    #[test]
    fn rejects_mandatory_column_colliding_with_reserved() {
        let mut cfg = config();
        cfg.mandatory
            .push(col("raw", ColumnTypeSpec::Scalar(ScalarType::String)));
        assert!(matches!(
            flexible_table("events", cfg, &SchemaLimits::default()),
            Err(SchemaError::ReservedColumn(_))
        ));
    }

    #[test]
    fn rejects_bad_table_name() {
        assert!(
            flexible_table("events; DROP TABLE x", config(), &SchemaLimits::default()).is_err()
        );
    }

    #[test]
    fn rejects_bad_column_name() {
        let mut cfg = config();
        cfg.mandatory[0].name = "org id".into();
        assert!(matches!(
            flexible_table("events", cfg, &SchemaLimits::default()),
            Err(SchemaError::InvalidIdentifier { .. })
        ));
    }

    #[test]
    fn custom_reserved_set_overrides_default() {
        // With a custom reserved set, "attrs"/"raw" appended automatically still
        // render, and the caller's reserved name is what's enforced.
        let mut cfg = config();
        cfg.reserved = Some(vec!["secret".into()]);
        cfg.promoted
            .push(col("secret", ColumnTypeSpec::Scalar(ScalarType::String)));
        assert!(matches!(
            flexible_table("events", cfg, &SchemaLimits::default()),
            Err(SchemaError::ReservedColumn(_))
        ));
    }
}
