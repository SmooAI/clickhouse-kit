//! TS→Rust bridge codegen. TypeScript owns the (static) schema; this turns a
//! ClickHouse table's live/spec columns into a Rust **row struct** — `#[derive(Row,
//! Deserialize)]` — so the Rust services get faithful, drift-checked rows for the
//! TS-authored tables instead of hand-writing them (the class of bug that bit
//! api-prime's hand-copied structs). Pair with `introspect` + `check_drift`: the
//! generated rows are asserted ≡ the live ClickHouse in CI.
//!
//! The mapping is a faithful **scaffold**: ClickHouse temporal types map to
//! `String` (the works-everywhere default over the HTTP/RowBinary boundary) — a
//! consumer may refine those to `time`/`chrono` types behind the `clickhouse`
//! crate's feature flags.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
