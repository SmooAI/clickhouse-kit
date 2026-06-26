//! Safe-by-construction primitives for user-defined / multi-tenant ClickHouse
//! schemas — the Rust-canonical port of `@smooai/clickhouse-kit`'s safety core.
//!
//! When column names + types come from untrusted input (a customer config, a DB
//! row, JSON), these make SQL injection and unbounded tables impossible on the
//! happy path. In Rust the type allowlist is even stronger than the TS version:
//! disallowed types (`Decimal`, `FixedString`, `Tuple`, …) have **no representation**
//! in [`ColumnTypeSpec`], so untrusted input naming them fails to deserialize.

use serde::Deserialize;

/// Raised when untrusted schema input violates a safety rule.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("empty {0} name")]
    EmptyIdentifier(&'static str),
    #[error("{kind} name too long: {len} > {max}")]
    IdentifierTooLong {
        kind: &'static str,
        len: usize,
        max: usize,
    },
    #[error("invalid {kind} name {name:?}: must match ^[A-Za-z_][A-Za-z0-9_]*$")]
    InvalidIdentifier { kind: &'static str, name: String },
    #[error("a table must declare at least one column")]
    NoColumns,
    #[error("too many columns: {count} > {max}")]
    TooManyColumns { count: usize, max: usize },
    #[error("column name {0:?} is reserved")]
    ReservedColumn(String),
}

/// Size bounds for a schema.
#[derive(Debug, Clone, Copy)]
pub struct SchemaLimits {
    pub max_columns: usize,
    pub max_identifier_length: usize,
}

impl Default for SchemaLimits {
    fn default() -> Self {
        Self {
            max_columns: 1024,
            max_identifier_length: 128,
        }
    }
}

/// Columns reserved for the flexible/hybrid table shape (catch-all + raw payload).
pub const DEFAULT_RESERVED_COLUMNS: &[&str] = &["attrs", "raw"];

fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validate a table/column identifier against the strict ASCII allowlist + length
/// bound. `kind` is `"table"` / `"column"` / `"identifier"` for error messages.
pub fn validate_identifier<'a>(
    name: &'a str,
    kind: &'static str,
    limits: &SchemaLimits,
) -> Result<&'a str, SchemaError> {
    if name.is_empty() {
        return Err(SchemaError::EmptyIdentifier(kind));
    }
    if name.len() > limits.max_identifier_length {
        return Err(SchemaError::IdentifierTooLong {
            kind,
            len: name.len(),
            max: limits.max_identifier_length,
        });
    }
    if !is_valid_identifier(name) {
        return Err(SchemaError::InvalidIdentifier {
            kind,
            name: name.to_string(),
        });
    }
    Ok(name)
}

/// Backtick-quote an identifier, escaping embedded backticks (defense-in-depth).
pub fn quote_identifier(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Error unless `count` is within the column-count bound.
pub fn assert_column_count(count: usize, limits: &SchemaLimits) -> Result<(), SchemaError> {
    if count < 1 {
        return Err(SchemaError::NoColumns);
    }
    if count > limits.max_columns {
        return Err(SchemaError::TooManyColumns {
            count,
            max: limits.max_columns,
        });
    }
    Ok(())
}

/// Error if `name` is reserved.
pub fn assert_not_reserved(name: &str, reserved: &[&str]) -> Result<(), SchemaError> {
    if reserved.contains(&name) {
        return Err(SchemaError::ReservedColumn(name.to_string()));
    }
    Ok(())
}

// ── Type allowlist ───────────────────────────────────────────────────────────

/// The allowlisted scalar column types. Anything else (Decimal, FixedString,
/// Tuple, Enum, Nested, …) has no variant, so it cannot be constructed and
/// untrusted input naming it fails to deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ScalarType {
    String,
    #[serde(rename = "UUID")]
    Uuid,
    Bool,
    Date,
    DateTime,
    DateTime64,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    #[serde(rename = "JSON")]
    Json,
}

impl ScalarType {
    fn ch_type(self) -> &'static str {
        match self {
            ScalarType::String => "String",
            ScalarType::Uuid => "UUID",
            ScalarType::Bool => "Bool",
            ScalarType::Date => "Date",
            ScalarType::DateTime => "DateTime",
            ScalarType::DateTime64 => "DateTime64(3)",
            ScalarType::Int8 => "Int8",
            ScalarType::Int16 => "Int16",
            ScalarType::Int32 => "Int32",
            ScalarType::Int64 => "Int64",
            ScalarType::UInt8 => "UInt8",
            ScalarType::UInt16 => "UInt16",
            ScalarType::UInt32 => "UInt32",
            ScalarType::UInt64 => "UInt64",
            ScalarType::Float32 => "Float32",
            ScalarType::Float64 => "Float64",
            ScalarType::Json => "JSON",
        }
    }
}

/// `String` is the only allowed `Array`/`Map` element type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum StringOnly {
    String,
}

/// A column type as supplied by untrusted input — the allowlisted recursive shape.
/// Mirrors the TS `ColumnTypeSpec`: a bare scalar string, or a single-key wrapper
/// object (`nullable` / `lowCardinality` / `array` / `map`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ColumnTypeSpec {
    Scalar(ScalarType),
    Nullable {
        nullable: Box<ColumnTypeSpec>,
    },
    LowCardinality {
        #[serde(rename = "lowCardinality")]
        low_cardinality: Box<ColumnTypeSpec>,
    },
    Array {
        array: StringOnly,
    },
    Map {
        map: (StringOnly, StringOnly),
    },
}

impl ColumnTypeSpec {
    /// The ClickHouse type string for this spec.
    pub fn to_ch_type(&self) -> String {
        match self {
            ColumnTypeSpec::Scalar(s) => s.ch_type().to_string(),
            ColumnTypeSpec::Nullable { nullable } => format!("Nullable({})", nullable.to_ch_type()),
            ColumnTypeSpec::LowCardinality { low_cardinality } => {
                format!("LowCardinality({})", low_cardinality.to_ch_type())
            }
            ColumnTypeSpec::Array { .. } => "Array(String)".to_string(),
            ColumnTypeSpec::Map { .. } => "Map(String, String)".to_string(),
        }
    }

    /// Whether a `DateTime64` is at the core (so a TTL move expression must wrap it
    /// in `toDateTime(...)`). Propagates through `Nullable`/`LowCardinality`.
    pub fn is_datetime64(&self) -> bool {
        match self {
            ColumnTypeSpec::Scalar(ScalarType::DateTime64) => true,
            ColumnTypeSpec::Nullable { nullable } => nullable.is_datetime64(),
            ColumnTypeSpec::LowCardinality { low_cardinality } => low_cardinality.is_datetime64(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> SchemaLimits {
        SchemaLimits::default()
    }

    #[test]
    fn accepts_safe_identifiers() {
        for ok in ["a", "A", "_x", "org_id", "col1", "X_2_y"] {
            assert_eq!(validate_identifier(ok, "column", &limits()).unwrap(), ok);
        }
    }

    #[test]
    fn rejects_injection_and_metacharacters() {
        let attacks = [
            "a; DROP TABLE x",
            "a`,`b",
            "a) ENGINE=Memory AS SELECT * FROM secrets --",
            "a' OR '1'='1",
            "a b",
            "a.b",
            "a-b",
            "1col",
            "",
            "a\"b",
            "a\nb",
            "таблица",
            "a/*x*/",
        ];
        for bad in attacks {
            assert!(
                validate_identifier(bad, "column", &limits()).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn enforces_length_bound() {
        let lim = limits();
        let too_long = "a".repeat(lim.max_identifier_length + 1);
        assert!(validate_identifier(&too_long, "column", &lim).is_err());
        let ok = "a".repeat(lim.max_identifier_length);
        assert!(validate_identifier(&ok, "column", &lim).is_ok());
    }

    #[test]
    fn quotes_and_escapes() {
        assert_eq!(quote_identifier("org_id"), "`org_id`");
        assert_eq!(quote_identifier("a`b"), "`a``b`");
    }

    #[test]
    fn bounds_and_reserved() {
        assert!(assert_column_count(0, &limits()).is_err());
        assert!(assert_column_count(limits().max_columns + 1, &limits()).is_err());
        assert!(assert_column_count(10, &limits()).is_ok());
        assert!(assert_not_reserved("attrs", DEFAULT_RESERVED_COLUMNS).is_err());
        assert!(assert_not_reserved("raw", DEFAULT_RESERVED_COLUMNS).is_err());
        assert!(assert_not_reserved("user_col", DEFAULT_RESERVED_COLUMNS).is_ok());
    }

    #[test]
    fn allowlist_builds_allowed_types() {
        let s: ColumnTypeSpec = serde_json::from_str("\"DateTime64\"").unwrap();
        assert_eq!(s.to_ch_type(), "DateTime64(3)");
        assert!(s.is_datetime64());

        let n: ColumnTypeSpec = serde_json::from_str(r#"{"nullable":"String"}"#).unwrap();
        assert_eq!(n.to_ch_type(), "Nullable(String)");

        let lc: ColumnTypeSpec =
            serde_json::from_str(r#"{"lowCardinality":{"nullable":"String"}}"#).unwrap();
        assert_eq!(lc.to_ch_type(), "LowCardinality(Nullable(String))");
        let lcd: ColumnTypeSpec =
            serde_json::from_str(r#"{"lowCardinality":"DateTime64"}"#).unwrap();
        assert!(lcd.is_datetime64());

        let a: ColumnTypeSpec = serde_json::from_str(r#"{"array":"String"}"#).unwrap();
        assert_eq!(a.to_ch_type(), "Array(String)");
        let m: ColumnTypeSpec = serde_json::from_str(r#"{"map":["String","String"]}"#).unwrap();
        assert_eq!(m.to_ch_type(), "Map(String, String)");
    }

    #[test]
    fn allowlist_rejects_disallowed_types() {
        let bad = [
            "\"Decimal(38, 10)\"",
            "\"FixedString(16)\"",
            "\"Enum8\"",
            "\"Tuple\"",
            "\"Nested\"",
            r#"{"map":["String","Int32"]}"#,
            r#"{"array":"Int32"}"#,
            r#"{"array":{"nullable":"String"}}"#,
            r#"{"wat":"String"}"#,
            "42",
        ];
        for b in bad {
            assert!(
                serde_json::from_str::<ColumnTypeSpec>(b).is_err(),
                "should reject {b}"
            );
        }
    }
}
