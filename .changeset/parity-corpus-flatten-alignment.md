---
"@smooai/clickhouse-kit": minor
---

Cross-language parity corpus, and two flatten behaviours it caught.

`spec/parity-corpus.json` is now loaded by **both** test suites (`src/__tests__/parity.test.ts` and `crates/clickhouse-kit/tests/parity.rs`), covering flatten, the column-type allowlist, and identifier validation — 57 shared cases. Previously each language asserted its own behaviour and nothing compared them, so a disagreement could sit indefinitely with both suites green. It had:

- **`maxDepth` counted differently.** TS reached `{"a.b": "{\"c\":1}"}` at `maxDepth: 1`; Rust needed `max_depth: 2` for the identical input and output, because it counted the root object as a level of descent. Rust now matches the TypeScript reference: `maxDepth` is levels of recursion **below** the root, so `maxDepth: 0` makes every value of the root a leaf. If you passed an explicit `max_depth` to the Rust `flatten_record`, subtract one to keep the previous behaviour; the default of 8 now descends one level further.
- **`maxKeys` truncation kept different keys.** TS iterated in insertion order, Rust in `serde_json::Map` order. Both now visit a node's keys in sorted order, so which keys survive a truncation is deterministic and identical — and no longer hostage to whether something in the dependency graph enables serde_json's `preserve_order`.

The type allowlist and identifier validation were checked against the same corpus and already agreed exactly; the corpus now holds them there.

`.github/workflows/rust.yml` gained `spec/**` to its path filter, so a corpus-only change runs the Rust suite instead of silently skipping it.
