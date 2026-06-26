//! Flatten + coerce — where arbitrary input meets a table.
//!
//! A schema is safe-by-construction (see [`crate::table`]), but the rows still
//! arrive as untrusted JSON whose shape rarely lines up with the columns. This
//! module bridges the two: [`flatten_record`] turns a nested document into a
//! bounded flat string map, and [`coerce_to_table`] routes a document's keys to
//! their matching columns, sweeping everything else into the `attrs` catch-all
//! (and the verbatim payload into `raw`). Neither does type validation — that
//! stays with the schema layer; this only decides *where* each value lands.

use crate::safety::DEFAULT_RESERVED_COLUMNS;
use crate::table::TableSpec;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// Bounds for [`flatten_record`]. The caps keep an adversarial document from
/// producing an unbounded key explosion.
#[derive(Debug, Clone)]
pub struct FlattenOptions {
    /// Maximum object nesting to descend into; deeper objects are kept whole as
    /// a JSON-stringified leaf rather than recursed.
    pub max_depth: usize,
    /// Hard ceiling on the number of flattened keys produced.
    pub max_keys: usize,
    /// Separator joining a nested path into a dotted key (e.g. `a.b.c`).
    pub delimiter: String,
}

impl Default for FlattenOptions {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_keys: 1024,
            delimiter: ".".to_string(),
        }
    }
}

/// Stringify a non-recursable JSON value: strings pass through unquoted, every
/// other shape (numbers, bools, null, arrays, and depth-capped objects) is its
/// compact JSON form.
fn stringify_leaf(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn flatten_into(
    prefix: &str,
    value: &Value,
    depth: usize,
    opts: &FlattenOptions,
    out: &mut BTreeMap<String, String>,
) {
    if out.len() >= opts.max_keys {
        return;
    }
    match value {
        // Descend into objects until the depth cap; arrays are never recursed.
        Value::Object(map) if depth < opts.max_depth => {
            for (k, v) in map {
                if out.len() >= opts.max_keys {
                    break;
                }
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}{}{k}", opts.delimiter)
                };
                flatten_into(&key, v, depth + 1, opts, out);
            }
        }
        // Leaf: a primitive, an array, or an object at/over the depth cap. A
        // top-level leaf (empty prefix) has no key to live under, so it is dropped.
        _ => {
            if !prefix.is_empty() {
                out.insert(prefix.to_string(), stringify_leaf(value));
            }
        }
    }
}

/// Flatten a JSON document into a bounded `dotted.key -> string` map.
///
/// Nested objects become dotted keys; arrays are JSON-stringified (not recursed);
/// primitives are stringified. Recursion stops at `max_depth` (deeper objects are
/// kept whole as a JSON string) and the result never exceeds `max_keys` entries.
/// Pure — no allocation beyond the returned map.
pub fn flatten_record(value: &Value, opts: &FlattenOptions) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    flatten_into("", value, 0, opts, &mut out);
    out
}

/// The outcome of coercing a document onto a [`TableSpec`].
#[derive(Debug, Clone)]
pub struct CoerceResult {
    /// Column-name → value, ready to bind. Matched keys pass through verbatim;
    /// `attrs`/`raw` (when present on the table) hold the catch-all/payload.
    pub row: BTreeMap<String, Value>,
    /// Input keys that matched no column and were swept into `attrs`.
    pub overflow_keys: Vec<String>,
}

/// Coerce an untrusted document onto a table's columns.
///
/// Each input key whose name matches a (non-reserved) column is placed in `row`
/// under that column, value passed through untouched — no type validation here.
/// Every unmatched key is recorded in `overflow_keys` and, if the table has an
/// `attrs` catch-all column, swept into it as a flattened string map (stored as a
/// JSON object `Value`). If the table has a `raw` String column, the verbatim
/// document is stored there as a JSON string.
pub fn coerce_to_table(input: Value, table: &TableSpec, opts: &FlattenOptions) -> CoerceResult {
    // `attrs`/`raw` are managed columns — input keys never land in them directly.
    let reserved: HashSet<&str> = DEFAULT_RESERVED_COLUMNS.iter().copied().collect();
    let matchable: HashSet<&str> = table
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .filter(|n| !reserved.contains(n))
        .collect();
    let has_attrs = table.columns.iter().any(|c| c.name == "attrs");
    let has_raw = table.columns.iter().any(|c| c.name == "raw");

    let mut row: BTreeMap<String, Value> = BTreeMap::new();
    let mut overflow_keys: Vec<String> = Vec::new();

    if has_raw {
        row.insert("raw".to_string(), Value::String(input.to_string()));
    }

    if let Value::Object(map) = input {
        let mut leftover = serde_json::Map::new();
        for (k, v) in map {
            if matchable.contains(k.as_str()) {
                row.insert(k, v);
            } else {
                overflow_keys.push(k.clone());
                leftover.insert(k, v);
            }
        }
        if has_attrs && !leftover.is_empty() {
            let flat = flatten_record(&Value::Object(leftover), opts);
            let attrs: serde_json::Map<String, Value> = flat
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            row.insert("attrs".to_string(), Value::Object(attrs));
        }
    }

    CoerceResult { row, overflow_keys }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{ColumnTypeSpec, ScalarType, StringOnly};
    use crate::table::{ColumnSpec, TableSpec};
    use serde_json::json;

    fn col(name: &str, type_spec: ColumnTypeSpec) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            type_spec,
            default: None,
        }
    }

    /// Table with two real columns plus the `attrs` Map catch-all and `raw` String.
    fn fixture() -> TableSpec {
        TableSpec {
            name: "events".into(),
            columns: vec![
                col("id", ColumnTypeSpec::Scalar(ScalarType::Uuid)),
                col("name", ColumnTypeSpec::Scalar(ScalarType::String)),
                col(
                    "attrs",
                    ColumnTypeSpec::Map {
                        map: (StringOnly::String, StringOnly::String),
                    },
                ),
                col("raw", ColumnTypeSpec::Scalar(ScalarType::String)),
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
    fn flattens_nested_objects_to_dotted_keys() {
        let v = json!({ "a": { "b": { "c": 1 } }, "d": "x" });
        let flat = flatten_record(&v, &FlattenOptions::default());
        assert_eq!(flat.get("a.b.c").map(String::as_str), Some("1"));
        assert_eq!(flat.get("d").map(String::as_str), Some("x"));
    }

    #[test]
    fn stringifies_arrays_and_primitives_without_recursing() {
        let v = json!({
            "tags": ["x", "y"],
            "n": 42,
            "f": 1.5,
            "b": true,
            "z": null,
            "s": "hello",
        });
        let flat = flatten_record(&v, &FlattenOptions::default());
        // Array kept whole as a JSON string, not flattened into tags.0 / tags.1.
        assert_eq!(flat.get("tags").map(String::as_str), Some(r#"["x","y"]"#));
        assert!(!flat.keys().any(|k| k.starts_with("tags.")));
        // Strings pass through unquoted; everything else is its JSON form.
        assert_eq!(flat.get("s").map(String::as_str), Some("hello"));
        assert_eq!(flat.get("n").map(String::as_str), Some("42"));
        assert_eq!(flat.get("f").map(String::as_str), Some("1.5"));
        assert_eq!(flat.get("b").map(String::as_str), Some("true"));
        assert_eq!(flat.get("z").map(String::as_str), Some("null"));
    }

    #[test]
    fn enforces_depth_cap() {
        let v = json!({ "a": { "b": { "c": 1 } } });
        let opts = FlattenOptions {
            max_depth: 2,
            ..FlattenOptions::default()
        };
        let flat = flatten_record(&v, &opts);
        // Recurses a -> b, then keeps the depth-capped object whole.
        assert_eq!(flat.get("a.b").map(String::as_str), Some(r#"{"c":1}"#));
        assert!(!flat.keys().any(|k| k == "a.b.c"));
    }

    #[test]
    fn enforces_key_cap() {
        let v = json!({ "a": 1, "b": 2, "c": 3, "d": 4 });
        let opts = FlattenOptions {
            max_keys: 2,
            ..FlattenOptions::default()
        };
        let flat = flatten_record(&v, &opts);
        assert_eq!(flat.len(), 2);
    }

    #[test]
    fn honors_custom_delimiter() {
        let v = json!({ "a": { "b": 1 } });
        let opts = FlattenOptions {
            delimiter: "__".to_string(),
            ..FlattenOptions::default()
        };
        let flat = flatten_record(&v, &opts);
        assert_eq!(flat.get("a__b").map(String::as_str), Some("1"));
    }

    #[test]
    fn coerce_routes_known_keys_to_columns() {
        let input = json!({ "id": "abc", "name": "widget" });
        let res = coerce_to_table(input, &fixture(), &FlattenOptions::default());
        assert_eq!(res.row.get("id"), Some(&json!("abc")));
        assert_eq!(res.row.get("name"), Some(&json!("widget")));
        assert!(res.overflow_keys.is_empty());
        // No unmatched keys → no attrs entry.
        assert!(!res.row.contains_key("attrs"));
    }

    #[test]
    fn coerce_sweeps_unknown_keys_into_attrs() {
        let input = json!({
            "id": "abc",
            "extra": { "nested": "v" },
            "color": "blue",
        });
        let res = coerce_to_table(input, &fixture(), &FlattenOptions::default());

        // Known key passed through.
        assert_eq!(res.row.get("id"), Some(&json!("abc")));

        // Unknown keys recorded as overflow.
        let mut overflow = res.overflow_keys.clone();
        overflow.sort();
        assert_eq!(overflow, vec!["color".to_string(), "extra".to_string()]);

        // attrs holds the flattened leftover as a JSON object of strings.
        let attrs = res.row.get("attrs").expect("attrs populated");
        assert_eq!(attrs, &json!({ "extra.nested": "v", "color": "blue" }));
    }

    #[test]
    fn coerce_sets_raw_payload() {
        let input = json!({ "id": "abc", "color": "blue" });
        let res = coerce_to_table(input.clone(), &fixture(), &FlattenOptions::default());
        assert_eq!(res.row.get("raw"), Some(&Value::String(input.to_string())));
    }

    #[test]
    fn coerce_without_catch_all_columns_drops_overflow_to_keys_only() {
        let table = TableSpec {
            name: "plain".into(),
            columns: vec![col("id", ColumnTypeSpec::Scalar(ScalarType::String))],
            engine: "MergeTree()".into(),
            order_by: vec!["id".into()],
            partition_by: None,
            ttl: None,
            indexes: vec![],
            settings: vec![],
        };
        let input = json!({ "id": "abc", "extra": "x" });
        let res = coerce_to_table(input, &table, &FlattenOptions::default());
        assert_eq!(res.row.get("id"), Some(&json!("abc")));
        assert_eq!(res.overflow_keys, vec!["extra".to_string()]);
        // No attrs/raw columns → nothing extra in the row.
        assert!(!res.row.contains_key("attrs"));
        assert!(!res.row.contains_key("raw"));
        assert_eq!(res.row.len(), 1);
    }
}
