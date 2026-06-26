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

/// A table built from a runtime spec. `engine` is app-controlled (not user input);
/// `order_by` entries are validated as column identifiers.
#[derive(Debug, Clone)]
pub struct TableSpec {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    pub engine: String,
    pub order_by: Vec<String>,
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

    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n)\nENGINE = {}\nORDER BY ({})",
        table.name,
        col_lines.join(",\n"),
        table.engine,
        table.order_by.join(", "),
    ))
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
}
