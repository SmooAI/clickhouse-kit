//! Additive, bounded schema evolution — grow a dynamic per-tenant table to match
//! its [`TableSpec`] without ever dropping or modifying existing columns.
//!
//! This path is **additive only**: it reports columns the kit declares that the
//! live table is missing, and emits `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`
//! for each. Live-only columns and type differences are intentionally ignored —
//! removing or retyping a tenant's column is never this path's job. The added
//! column's type comes from the kit's own (trusted) [`ColumnTypeSpec::to_ch_type`],
//! never from the introspected live shape.

use crate::safety::quote_identifier;
use crate::table::TableSpec;

/// An introspected column from the live table (e.g. from `system.columns`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveColumn {
    pub name: String,
    pub type_name: String,
}

/// A column the kit declares that the live table is missing. `ch_type` is the
/// trusted ClickHouse type derived from the kit's [`TableSpec`], not from `live`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDiff {
    pub name: String,
    pub ch_type: String,
}

/// Columns present in `table` but absent from `live` (matched by name).
///
/// Reports **only** missing (kit-but-not-live) columns. Live-only columns and
/// type differences are ignored — they are not this additive path's concern.
/// Each [`ColumnDiff::ch_type`] is computed from the table column's own trusted
/// [`ColumnTypeSpec::to_ch_type`], never from the live introspection.
pub fn diff_columns(table: &TableSpec, live: &[LiveColumn]) -> Vec<ColumnDiff> {
    table
        .columns
        .iter()
        .filter(|c| !live.iter().any(|l| l.name == c.name))
        .map(|c| ColumnDiff {
            name: c.name.clone(),
            ch_type: c.type_spec.to_ch_type(),
        })
        .collect()
}

/// One `ALTER TABLE <table> ADD COLUMN IF NOT EXISTS <col> <ch_type>` per missing
/// column. The table and column names are backtick-quoted via [`quote_identifier`]
/// (defense-in-depth); the type is the trusted [`ColumnDiff::ch_type`]. Returns an
/// empty vec when nothing is missing.
pub fn alter_add_columns_sql(table: &TableSpec, missing: &[ColumnDiff]) -> Vec<String> {
    let quoted_table = quote_identifier(&table.name);
    missing
        .iter()
        .map(|d| {
            format!(
                "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {}",
                quoted_table,
                quote_identifier(&d.name),
                d.ch_type,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{ColumnTypeSpec, ScalarType};
    use crate::table::ColumnSpec;

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
            ],
            engine: "MergeTree()".into(),
            order_by: vec!["id".into()],
        }
    }

    fn live(name: &str, type_name: &str) -> LiveColumn {
        LiveColumn {
            name: name.into(),
            type_name: type_name.into(),
        }
    }

    #[test]
    fn diff_finds_only_missing_columns() {
        let table = sample();
        // `id` is present; `ts` and `name` are missing. `extra` is live-only.
        let live = [live("id", "UUID"), live("extra", "String")];
        let diff = diff_columns(&table, &live);
        assert_eq!(
            diff,
            vec![
                ColumnDiff {
                    name: "ts".into(),
                    ch_type: "DateTime64(3)".into(),
                },
                ColumnDiff {
                    name: "name".into(),
                    ch_type: "String".into(),
                },
            ]
        );
    }

    #[test]
    fn diff_ignores_live_only_and_retyped_columns() {
        let table = sample();
        // Every kit column is present live, but `id`/`ts`/`name` carry *different*
        // live types, and there's a live-only `extra`. Type differences are not
        // this path's concern, so the diff must be empty.
        let live = [
            live("id", "String"),
            live("ts", "DateTime"),
            live("name", "Int64"),
            live("extra", "String"),
        ];
        assert!(diff_columns(&table, &live).is_empty());
    }

    #[test]
    fn diff_ch_type_comes_from_kit_not_live() {
        let table = sample();
        // `ts` is missing live; its ch_type must be the kit's trusted DateTime64(3),
        // regardless of any live type that might claim otherwise.
        let live = [live("id", "UUID"), live("name", "String")];
        let diff = diff_columns(&table, &live);
        assert_eq!(
            diff,
            vec![ColumnDiff {
                name: "ts".into(),
                ch_type: "DateTime64(3)".into(),
            }]
        );
    }

    #[test]
    fn alter_emits_add_column_if_not_exists_with_quoted_identifiers() {
        let table = sample();
        let missing = vec![
            ColumnDiff {
                name: "ts".into(),
                ch_type: "DateTime64(3)".into(),
            },
            ColumnDiff {
                name: "name".into(),
                ch_type: "String".into(),
            },
        ];
        let sql = alter_add_columns_sql(&table, &missing);
        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `events` ADD COLUMN IF NOT EXISTS `ts` DateTime64(3)".to_string(),
                "ALTER TABLE `events` ADD COLUMN IF NOT EXISTS `name` String".to_string(),
            ]
        );
    }

    #[test]
    fn alter_backtick_quotes_table_and_column() {
        let table = sample();
        let missing = vec![ColumnDiff {
            name: "name".into(),
            ch_type: "String".into(),
        }];
        let sql = alter_add_columns_sql(&table, &missing);
        assert_eq!(sql.len(), 1);
        assert!(sql[0].contains("ALTER TABLE `events`"));
        assert!(sql[0].contains("ADD COLUMN IF NOT EXISTS `name`"));
    }

    #[test]
    fn empty_when_in_sync() {
        let table = sample();
        let live = [
            live("id", "UUID"),
            live("ts", "DateTime64(3)"),
            live("name", "String"),
        ];
        let diff = diff_columns(&table, &live);
        assert!(diff.is_empty());
        assert!(alter_add_columns_sql(&table, &diff).is_empty());
    }
}
