//! Rust half of the cross-language parity corpus.
//!
//! Loads `spec/parity-corpus.json` — the SAME file `src/__tests__/parity.test.ts`
//! loads — so the two ports of flatten / the type allowlist / identifier validation
//! are checked against one committed set of expectations instead of two
//! hand-mirrored ones.
//!
//! That distinction is not academic. Before this corpus existed the two flatteners
//! disagreed about what `maxDepth` counts, and both suites were green: TS reached
//! `{"a.b": "{\"c\":1}"}` at `maxDepth` 1, Rust needed `max_depth` 2 for the identical
//! input and output, and each language's tests asserted its own answer. A case added
//! here must pass in every language or the build goes red.
//!
//! No `#![cfg(feature = "…")]` on this file, and no `#[ignore]`: `cargo test` runs it
//! by default. A parity suite that reports "0 passed; ok" is worse than none.

use clickhouse_kit::{
    flatten_record, validate_identifier, ColumnTypeSpec, FlattenOptions, SchemaLimits,
};
use serde_json::Value;
use std::collections::BTreeMap;

const CORPUS: &str = include_str!("../../../spec/parity-corpus.json");

fn corpus() -> Value {
    serde_json::from_str(CORPUS).expect("spec/parity-corpus.json is not valid JSON")
}

fn cases(root: &Value, section: &str) -> Vec<Value> {
    root[section]["cases"]
        .as_array()
        .unwrap_or_else(|| panic!("corpus section {section:?} has no `cases` array"))
        .clone()
}

/// A corpus nobody loads reads as a guarantee while proving nothing, and an empty
/// `cases` array would make every loop below silently vacuous.
#[test]
fn corpus_is_loaded_and_non_empty() {
    let root = corpus();
    assert!(cases(&root, "flatten").len() > 5);
    assert!(cases(&root, "columnTypes").len() > 15);
    assert!(cases(&root, "identifiers").len() > 10);
}

#[test]
fn flatten_matches_corpus() {
    for case in cases(&corpus(), "flatten") {
        let name = case["name"].as_str().unwrap();
        let opts = FlattenOptions {
            max_depth: case["options"]["maxDepth"].as_u64().unwrap() as usize,
            max_keys: case["options"]["maxKeys"].as_u64().unwrap() as usize,
            delimiter: case["options"]["delimiter"].as_str().unwrap().to_string(),
        };
        let expected: BTreeMap<String, String> =
            serde_json::from_value(case["expected"].clone()).unwrap();

        let actual = flatten_record(&case["input"], &opts);
        assert_eq!(actual, expected, "parity corpus — flatten: {name}");
    }
}

#[test]
fn column_types_match_corpus() {
    for case in cases(&corpus(), "columnTypes") {
        let name = case["name"].as_str().unwrap();
        let parsed = serde_json::from_value::<ColumnTypeSpec>(case["spec"].clone());

        if case["rejected"].as_bool().unwrap_or(false) {
            // Rust rejects a disallowed type by having no variant to deserialize
            // into; TS rejects it by throwing. Different mechanism, same guarantee.
            assert!(
                parsed.is_err(),
                "parity corpus — columnTypes: {name} should be rejected, but deserialized to {:?}",
                parsed.unwrap(),
            );
            continue;
        }

        let spec = parsed.unwrap_or_else(|e| {
            panic!("parity corpus — columnTypes: {name} should be accepted, got {e}")
        });
        assert_eq!(
            spec.to_ch_type(),
            case["chType"].as_str().unwrap(),
            "parity corpus — columnTypes: {name}",
        );
    }
}

#[test]
fn identifiers_match_corpus() {
    let limits = SchemaLimits::default();
    for case in cases(&corpus(), "identifiers") {
        let value = case["value"].as_str().unwrap();
        let valid = case["valid"].as_bool().unwrap();
        let why = case["why"].as_str().unwrap_or("");
        let got = validate_identifier(value, "column", &limits);
        assert_eq!(
            got.is_ok(),
            valid,
            "parity corpus — identifiers: {value:?} ({why}) — expected valid={valid}, got {got:?}",
        );
    }
}
