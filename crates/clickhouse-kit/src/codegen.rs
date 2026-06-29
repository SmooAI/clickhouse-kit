//! Codegen — emit typed bindings for a ClickHouse table in two directions:
//!
//! - **ClickHouse → Rust rows** (`ch_type_to_rust` / `rust_row_struct`): the
//!   TS→Rust bridge. TypeScript owns a (static) table; this turns its live/spec
//!   columns into a Rust `#[derive(Row)]` struct so the Rust services get faithful,
//!   drift-checked rows instead of hand-writing them. Pair with `introspect` +
//!   `check_drift`. Temporal types map to `String` (the works-everywhere default
//!   over the HTTP/RowBinary boundary; refine to `time`/`chrono` behind features).
//! - **`TableSpec` → TS + Zod** (`emit_row_interface` / `emit_select_schema` /
//!   `emit_insert_schema` / `emit_ts_module`): a `createSelectSchema`/`createInsertSchema`
//!   style emitter (parity with smooai-postgres-kit). Useful for handing a runtime
//!   (dynamic) table's shape to a TypeScript client as a row interface + Zod schemas.

use crate::safety::{ColumnTypeSpec, ScalarType};
use crate::table::{ColumnSpec, TableSpec};

// ── ClickHouse → Rust rows ─────────────────────────────────────────────────────

/// Strip a single-arg wrapper like `Nullable(...)` / `Array(...)`, returning the inner.
fn strip_wrapper<'a>(t: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}(");
    t.strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(')'))
}

/// Split a `Map(...)` inner on its top-level comma (respecting nested parens).
fn split_top_comma(inner: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return Some((inner[..i].trim(), inner[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// Map a ClickHouse type string to the Rust type a `clickhouse`-crate row uses.
/// Wrappers recurse; unknown scalars fall back to `String` (safe over the wire).
pub fn ch_type_to_rust(ch_type: &str) -> String {
    let t = ch_type.trim();
    if let Some(inner) = strip_wrapper(t, "Nullable") {
        return format!("Option<{}>", ch_type_to_rust(inner));
    }
    if let Some(inner) = strip_wrapper(t, "LowCardinality") {
        return ch_type_to_rust(inner);
    }
    if let Some(inner) = strip_wrapper(t, "Array") {
        return format!("Vec<{}>", ch_type_to_rust(inner));
    }
    if let Some(inner) = strip_wrapper(t, "Map") {
        if let Some((k, v)) = split_top_comma(inner) {
            return format!(
                "std::collections::HashMap<{}, {}>",
                ch_type_to_rust(k),
                ch_type_to_rust(v)
            );
        }
    }
    // Scalar — match on the base type, ignoring any `(...)` parameters.
    let base = t.split('(').next().unwrap_or(t).trim();
    match base {
        "Bool" => "bool",
        "UInt8" => "u8",
        "UInt16" => "u16",
        "UInt32" => "u32",
        "UInt64" => "u64",
        "Int8" => "i8",
        "Int16" => "i16",
        "Int32" => "i32",
        "Int64" => "i64",
        "Float32" => "f32",
        "Float64" => "f64",
        // String, UUID, FixedString, Date*, DateTime*, IPv4/6, Enum*, JSON, and
        // anything unrecognized → String (the safe over-the-wire default).
        _ => "String",
    }
    .to_string()
}

/// Rust raw-ident escape for column names that collide with Rust keywords.
fn rust_field_ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
        "where", "while", "async", "await", "dyn",
    ];
    if KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

/// Emit a Rust row struct for a table's columns — `(column_name, clickhouse_type)`
/// pairs. Derives the `clickhouse` crate's `Row` + serde, so it deserializes
/// straight from a query. The emitted source references `clickhouse::Row`
/// (a dev/consumer dependency); this function only produces the string.
pub fn rust_row_struct(struct_name: &str, columns: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str(
        "#[derive(Debug, Clone, clickhouse::Row, serde::Serialize, serde::Deserialize)]\n",
    );
    out.push_str(&format!("pub struct {struct_name} {{\n"));
    for (name, ch_type) in columns {
        let field = rust_field_ident(name);
        // Preserve the exact column name for (de)serialization when the field was escaped.
        if field != *name {
            out.push_str(&format!("    #[serde(rename = \"{name}\")]\n"));
        }
        out.push_str(&format!("    pub {field}: {},\n", ch_type_to_rust(ch_type)));
    }
    out.push_str("}\n");
    out
}

// ── TableSpec → TS + Zod ───────────────────────────────────────────────────────

/// `snake_case` / `kebab-case` → `camelCase` (e.g. `organization_id` → `organizationId`).
fn to_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    let mut first = true;
    for c in s.chars() {
        if c == '_' || c == '-' {
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
    use crate::safety::StringOnly;

    // ── ClickHouse → Rust ──
    #[test]
    fn maps_scalars() {
        assert_eq!(ch_type_to_rust("String"), "String");
        assert_eq!(ch_type_to_rust("UInt64"), "u64");
        assert_eq!(ch_type_to_rust("Int32"), "i32");
        assert_eq!(ch_type_to_rust("Float64"), "f64");
        assert_eq!(ch_type_to_rust("Bool"), "bool");
        assert_eq!(ch_type_to_rust("UUID"), "String");
        assert_eq!(ch_type_to_rust("DateTime64(3)"), "String");
    }

    #[test]
    fn maps_wrappers_and_containers() {
        assert_eq!(ch_type_to_rust("Nullable(String)"), "Option<String>");
        assert_eq!(ch_type_to_rust("LowCardinality(String)"), "String");
        assert_eq!(
            ch_type_to_rust("LowCardinality(Nullable(String))"),
            "Option<String>"
        );
        assert_eq!(ch_type_to_rust("Array(String)"), "Vec<String>");
        assert_eq!(ch_type_to_rust("Array(UInt32)"), "Vec<u32>");
        assert_eq!(
            ch_type_to_rust("Map(String, String)"),
            "std::collections::HashMap<String, String>"
        );
        assert_eq!(
            ch_type_to_rust("Map(String, Array(UInt8))"),
            "std::collections::HashMap<String, Vec<u8>>"
        );
    }

    #[test]
    fn emits_row_struct_with_keyword_escape() {
        let cols = vec![
            ("id".to_string(), "UUID".to_string()),
            ("count".to_string(), "UInt64".to_string()),
            ("type".to_string(), "LowCardinality(String)".to_string()),
            ("tags".to_string(), "Array(String)".to_string()),
        ];
        let src = rust_row_struct("EventRow", &cols);
        assert!(src.contains(
            "#[derive(Debug, Clone, clickhouse::Row, serde::Serialize, serde::Deserialize)]"
        ));
        assert!(src.contains("pub struct EventRow {"));
        assert!(src.contains("pub id: String,"));
        assert!(src.contains("pub count: u64,"));
        assert!(src.contains("#[serde(rename = \"type\")]"));
        assert!(src.contains("pub r#type: String,"));
        assert!(src.contains("pub tags: Vec<String>,"));
    }

    // ── TableSpec → TS / Zod ──
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

    fn sample() -> TableSpec {
        TableSpec {
            name: "events".into(),
            columns: vec![
                col("id", ColumnTypeSpec::Scalar(ScalarType::Uuid)),
                col(
                    "occurred_at",
                    ColumnTypeSpec::Scalar(ScalarType::DateTime64),
                ),
                col("status", lc(ColumnTypeSpec::Scalar(ScalarType::String))),
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
