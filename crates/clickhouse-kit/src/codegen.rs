//! TS + Zod code emit for a [`TableSpec`] — the Rust-canonical port of the retired
//! `@smooai/clickhouse-kit` `createSelectSchema`/`createInsertSchema` emitter.
//!
//! From a [`TableSpec`] this emits, mirroring the retired TS package's output style:
//! - a TS row `interface` (one field per column),
//! - a Zod **select** schema (`z.object(...)`), and
//! - a Zod **insert** schema (columns with a ClickHouse `DEFAULT` become `.optional()`).
//!
//! The ClickHouse-type → TS/Zod mapping:
//!
//! | ClickHouse                | TS                       | Zod                                  |
//! | ------------------------- | ------------------------ | ------------------------------------ |
//! | `String`                  | `string`                 | `z.string()`                         |
//! | `UUID`                    | `string`                 | `z.string()`                         |
//! | `Bool`                    | `boolean`                | `z.boolean()`                        |
//! | `Int*` / `UInt*`          | `number`                 | `z.number()`                         |
//! | `Float32` / `Float64`     | `number`                 | `z.number()`                         |
//! | `Date`/`DateTime`/`DateTime64` | `string`            | `z.string()`                         |
//! | `JSON`                    | `unknown`                | `z.unknown()`                        |
//! | `Nullable(T)`             | `T \| null` (optional `?`) | `<T>.nullable()`                   |
//! | `LowCardinality(T)`       | `T`                      | `<T>`                                |
//! | `Array(String)`           | `string[]`               | `z.array(z.string())`                |
//! | `Map(String, String)`     | `Record<string, string>` | `z.record(z.string(), z.string())`   |
//!
//! Keys are emitted in `camelCase` (ClickHouse columns are conventionally
//! `snake_case`), the type/schema names are derived from the table name, and the
//! output uses 4-space indentation for parity with how `postgres-kit` emits.

use crate::safety::{ColumnTypeSpec, ScalarType};
use crate::table::{ColumnSpec, TableSpec};

// ── Naming helpers ───────────────────────────────────────────────────────────

/// `snake_case` / `kebab-case` → `camelCase` (e.g. `organization_id` → `organizationId`).
fn to_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    let mut first = true;
    for c in s.chars() {
        if c == '_' || c == '-' {
            // Don't uppercase across a leading separator; keep it as a boundary.
            upper_next = !first;
            continue;
        }
        if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
        first = false;
    }
    out
}

/// `snake_case` → `PascalCase` (e.g. `observability_traces` → `ObservabilityTraces`).
fn to_pascal_case(s: &str) -> String {
    let camel = to_camel_case(s);
    let mut chars = camel.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => camel,
    }
}

// ── Type mapping ─────────────────────────────────────────────────────────────

fn scalar_ts(s: ScalarType) -> &'static str {
    match s {
        ScalarType::String
        | ScalarType::Uuid
        | ScalarType::Date
        | ScalarType::DateTime
        | ScalarType::DateTime64 => "string",
        ScalarType::Bool => "boolean",
        ScalarType::Int8
        | ScalarType::Int16
        | ScalarType::Int32
        | ScalarType::Int64
        | ScalarType::UInt8
        | ScalarType::UInt16
        | ScalarType::UInt32
        | ScalarType::UInt64
        | ScalarType::Float32
        | ScalarType::Float64 => "number",
        ScalarType::Json => "unknown",
    }
}

fn scalar_zod(s: ScalarType) -> &'static str {
    match s {
        ScalarType::String
        | ScalarType::Uuid
        | ScalarType::Date
        | ScalarType::DateTime
        | ScalarType::DateTime64 => "z.string()",
        ScalarType::Bool => "z.boolean()",
        ScalarType::Int8
        | ScalarType::Int16
        | ScalarType::Int32
        | ScalarType::Int64
        | ScalarType::UInt8
        | ScalarType::UInt16
        | ScalarType::UInt32
        | ScalarType::UInt64
        | ScalarType::Float32
        | ScalarType::Float64 => "z.number()",
        ScalarType::Json => "z.unknown()",
    }
}

/// The TS type for a column spec. `Nullable(T)` widens to `T | null`;
/// `LowCardinality(T)` is transparent (renders as `T`).
fn ts_type(spec: &ColumnTypeSpec) -> String {
    match spec {
        ColumnTypeSpec::Scalar(s) => scalar_ts(*s).to_string(),
        ColumnTypeSpec::DateTime64 { .. } => "string".to_string(),
        ColumnTypeSpec::Nullable { nullable } => format!("{} | null", ts_type(nullable)),
        ColumnTypeSpec::LowCardinality { low_cardinality } => ts_type(low_cardinality),
        ColumnTypeSpec::Array { .. } => "string[]".to_string(),
        ColumnTypeSpec::Map { .. } => "Record<string, string>".to_string(),
    }
}

/// The Zod expression for a column spec. `Nullable(T)` appends `.nullable()`;
/// `LowCardinality(T)` is transparent (renders as the inner Zod).
fn zod_type(spec: &ColumnTypeSpec) -> String {
    match spec {
        ColumnTypeSpec::Scalar(s) => scalar_zod(*s).to_string(),
        ColumnTypeSpec::DateTime64 { .. } => "z.string()".to_string(),
        ColumnTypeSpec::Nullable { nullable } => format!("{}.nullable()", zod_type(nullable)),
        ColumnTypeSpec::LowCardinality { low_cardinality } => zod_type(low_cardinality),
        ColumnTypeSpec::Array { .. } => "z.array(z.string())".to_string(),
        ColumnTypeSpec::Map { .. } => "z.record(z.string(), z.string())".to_string(),
    }
}

/// Whether a column is nullable (a `Nullable(...)` at the core, seen through any
/// transparent `LowCardinality(...)` wrappers). Nullable columns become optional
/// (`field?`) in the emitted interface.
fn is_nullable(spec: &ColumnTypeSpec) -> bool {
    match spec {
        ColumnTypeSpec::Nullable { .. } => true,
        ColumnTypeSpec::LowCardinality { low_cardinality } => is_nullable(low_cardinality),
        _ => false,
    }
}

// ── Emit ─────────────────────────────────────────────────────────────────────

/// The TS interface name for a table, e.g. `observability_traces` → `ObservabilityTracesRow`.
pub fn row_type_name(table: &TableSpec) -> String {
    format!("{}Row", to_pascal_case(&table.name))
}

/// The Zod select-schema const name, e.g. `observabilityTracesSelectSchema`.
pub fn select_schema_name(table: &TableSpec) -> String {
    format!("{}SelectSchema", to_camel_case(&table.name))
}

/// The Zod insert-schema const name, e.g. `observabilityTracesInsertSchema`.
pub fn insert_schema_name(table: &TableSpec) -> String {
    format!("{}InsertSchema", to_camel_case(&table.name))
}

/// Emit the TS row `interface` for a table (one field per column).
pub fn emit_row_interface(table: &TableSpec) -> String {
    let mut out = format!("export interface {} {{\n", row_type_name(table));
    for c in &table.columns {
        let optional = if is_nullable(&c.type_spec) { "?" } else { "" };
        out.push_str(&format!(
            "    {}{}: {};\n",
            to_camel_case(&c.name),
            optional,
            ts_type(&c.type_spec)
        ));
    }
    out.push('}');
    out
}

fn emit_zod_object(name: &str, columns: &[ColumnSpec], insert: bool) -> String {
    let mut out = format!("export const {name} = z.object({{\n");
    for c in columns {
        let mut zod = zod_type(&c.type_spec);
        // Columns with a ClickHouse DEFAULT are optional on insert (the server fills them).
        if insert && c.default.is_some() {
            zod.push_str(".optional()");
        }
        out.push_str(&format!("    {}: {},\n", to_camel_case(&c.name), zod));
    }
    out.push_str("});");
    out
}

/// Emit the Zod **select** schema (`z.object(...)`) for a table.
pub fn emit_select_schema(table: &TableSpec) -> String {
    emit_zod_object(&select_schema_name(table), &table.columns, false)
}

/// Emit the Zod **insert** schema — columns with a `DEFAULT` become `.optional()`.
pub fn emit_insert_schema(table: &TableSpec) -> String {
    emit_zod_object(&insert_schema_name(table), &table.columns, true)
}

/// Emit a full TS module for a table: the `zod` import, the row interface, and the
/// select + insert schemas, separated by blank lines.
pub fn emit_ts_module(table: &TableSpec) -> String {
    format!(
        "import {{ z }} from \"zod\";\n\n{}\n\n{}\n\n{}\n",
        emit_row_interface(table),
        emit_select_schema(table),
        emit_insert_schema(table),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{ScalarType, StringOnly};

    fn col(name: &str, t: ColumnTypeSpec) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            type_spec: t,
            default: None,
        }
    }

    fn lc(inner: ColumnTypeSpec) -> ColumnTypeSpec {
        ColumnTypeSpec::LowCardinality {
            low_cardinality: Box::new(inner),
        }
    }

    fn nullable(inner: ColumnTypeSpec) -> ColumnTypeSpec {
        ColumnTypeSpec::Nullable {
            nullable: Box::new(inner),
        }
    }

    /// A representative table covering every interesting case: scalar, UUID,
    /// DateTime64, Bool, numeric, `Array(String)`, `Map(String, String)`, JSON,
    /// enum-ish `LowCardinality(String)`, `LowCardinality(Nullable(String))`, and a
    /// `DEFAULT`-bearing column (optional on insert).
    fn sample() -> TableSpec {
        TableSpec {
            name: "events".into(),
            columns: vec![
                col("id", ColumnTypeSpec::Scalar(ScalarType::Uuid)),
                col(
                    "occurred_at",
                    ColumnTypeSpec::Scalar(ScalarType::DateTime64),
                ),
                // enum-ish LowCardinality(String)
                col("status", lc(ColumnTypeSpec::Scalar(ScalarType::String))),
                // nullable, through a LowCardinality wrapper
                col(
                    "region",
                    lc(nullable(ColumnTypeSpec::Scalar(ScalarType::String))),
                ),
                col("score", ColumnTypeSpec::Scalar(ScalarType::Float64)),
                col("retry_count", ColumnTypeSpec::Scalar(ScalarType::UInt32)),
                col("is_error", ColumnTypeSpec::Scalar(ScalarType::Bool)),
                col(
                    "tags",
                    ColumnTypeSpec::Array {
                        array: StringOnly::String,
                    },
                ),
                col(
                    "attributes",
                    ColumnTypeSpec::Map {
                        map: (StringOnly::String, StringOnly::String),
                    },
                ),
                col("payload", ColumnTypeSpec::Scalar(ScalarType::Json)),
                ColumnSpec {
                    name: "ingested_at".into(),
                    type_spec: ColumnTypeSpec::Scalar(ScalarType::DateTime),
                    default: Some("now()".into()),
                },
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
    fn names_are_derived_from_table_name() {
        let t = TableSpec {
            name: "observability_traces".into(),
            ..sample()
        };
        assert_eq!(row_type_name(&t), "ObservabilityTracesRow");
        assert_eq!(select_schema_name(&t), "observabilityTracesSelectSchema");
        assert_eq!(insert_schema_name(&t), "observabilityTracesInsertSchema");
    }

    #[test]
    fn golden_row_interface() {
        let expected = "\
export interface EventsRow {
    id: string;
    occurredAt: string;
    status: string;
    region?: string | null;
    score: number;
    retryCount: number;
    isError: boolean;
    tags: string[];
    attributes: Record<string, string>;
    payload: unknown;
    ingestedAt: string;
}";
        assert_eq!(emit_row_interface(&sample()), expected);
    }

    #[test]
    fn golden_select_schema() {
        let expected = "\
export const eventsSelectSchema = z.object({
    id: z.string(),
    occurredAt: z.string(),
    status: z.string(),
    region: z.string().nullable(),
    score: z.number(),
    retryCount: z.number(),
    isError: z.boolean(),
    tags: z.array(z.string()),
    attributes: z.record(z.string(), z.string()),
    payload: z.unknown(),
    ingestedAt: z.string(),
});";
        assert_eq!(emit_select_schema(&sample()), expected);
    }

    #[test]
    fn golden_insert_schema_makes_default_columns_optional() {
        let expected = "\
export const eventsInsertSchema = z.object({
    id: z.string(),
    occurredAt: z.string(),
    status: z.string(),
    region: z.string().nullable(),
    score: z.number(),
    retryCount: z.number(),
    isError: z.boolean(),
    tags: z.array(z.string()),
    attributes: z.record(z.string(), z.string()),
    payload: z.unknown(),
    ingestedAt: z.string().optional(),
});";
        assert_eq!(emit_insert_schema(&sample()), expected);
    }

    #[test]
    fn golden_full_module() {
        let expected = "\
import { z } from \"zod\";

export interface EventsRow {
    id: string;
    occurredAt: string;
    status: string;
    region?: string | null;
    score: number;
    retryCount: number;
    isError: boolean;
    tags: string[];
    attributes: Record<string, string>;
    payload: unknown;
    ingestedAt: string;
}

export const eventsSelectSchema = z.object({
    id: z.string(),
    occurredAt: z.string(),
    status: z.string(),
    region: z.string().nullable(),
    score: z.number(),
    retryCount: z.number(),
    isError: z.boolean(),
    tags: z.array(z.string()),
    attributes: z.record(z.string(), z.string()),
    payload: z.unknown(),
    ingestedAt: z.string(),
});

export const eventsInsertSchema = z.object({
    id: z.string(),
    occurredAt: z.string(),
    status: z.string(),
    region: z.string().nullable(),
    score: z.number(),
    retryCount: z.number(),
    isError: z.boolean(),
    tags: z.array(z.string()),
    attributes: z.record(z.string(), z.string()),
    payload: z.unknown(),
    ingestedAt: z.string().optional(),
});
";
        assert_eq!(emit_ts_module(&sample()), expected);
    }

    #[test]
    fn parametrised_datetime64_maps_to_string() {
        let dt: ColumnTypeSpec =
            serde_json::from_str(r#"{"datetime64":{"precision":6,"timezone":"UTC"}}"#).unwrap();
        let t = TableSpec {
            name: "t".into(),
            columns: vec![col("occurred_at", dt)],
            ..sample()
        };
        assert!(emit_row_interface(&t).contains("occurredAt: string;"));
        assert!(emit_select_schema(&t).contains("occurredAt: z.string()"));
    }

    #[test]
    fn nullable_scalar_without_low_cardinality_is_optional_and_nullable() {
        let t = TableSpec {
            name: "t".into(),
            columns: vec![col(
                "note",
                nullable(ColumnTypeSpec::Scalar(ScalarType::String)),
            )],
            ..sample()
        };
        assert!(emit_row_interface(&t).contains("note?: string | null;"));
        assert!(emit_select_schema(&t).contains("note: z.string().nullable(),"));
    }

    #[test]
    fn camel_case_helper() {
        assert_eq!(to_camel_case("organization_id"), "organizationId");
        assert_eq!(to_camel_case("started_at"), "startedAt");
        assert_eq!(to_camel_case("id"), "id");
        assert_eq!(to_camel_case("_leading"), "leading");
        assert_eq!(
            to_pascal_case("observability_traces"),
            "ObservabilityTraces"
        );
    }
}
