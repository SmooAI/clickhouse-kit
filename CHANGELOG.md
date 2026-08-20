# @smooai/clickhouse-kit

## 0.5.0

### Minor Changes

- b42bdd0: Cross-language parity corpus, and two flatten behaviours it caught.

  `spec/parity-corpus.json` is now loaded by **both** test suites (`src/__tests__/parity.test.ts` and `crates/clickhouse-kit/tests/parity.rs`), covering flatten, the column-type allowlist, and identifier validation — 57 shared cases. Previously each language asserted its own behaviour and nothing compared them, so a disagreement could sit indefinitely with both suites green. It had:
  - **`maxDepth` counted differently.** TS reached `{"a.b": "{\"c\":1}"}` at `maxDepth: 1`; Rust needed `max_depth: 2` for the identical input and output, because it counted the root object as a level of descent. Rust now matches the TypeScript reference: `maxDepth` is levels of recursion **below** the root, so `maxDepth: 0` makes every value of the root a leaf. If you passed an explicit `max_depth` to the Rust `flatten_record`, subtract one to keep the previous behaviour; the default of 8 now descends one level further.
  - **`maxKeys` truncation kept different keys.** TS iterated in insertion order, Rust in `serde_json::Map` order. Both now visit a node's keys in sorted order, so which keys survive a truncation is deterministic and identical — and no longer hostage to whether something in the dependency graph enables serde_json's `preserve_order`.

  The type allowlist and identifier validation were checked against the same corpus and already agreed exactly; the corpus now holds them there.

  `.github/workflows/rust.yml` gained `spec/**` to its path filter, so a corpus-only change runs the Rust suite instead of silently skipping it.

## 0.4.0

### Minor Changes

- e2fac48: TS + Zod code emit in the Rust crate (`crates/clickhouse-kit/src/codegen.rs`) — from a `TableSpec`, emit a TS row `interface`, a Zod **select** schema, and a Zod **insert** schema (columns with a ClickHouse `DEFAULT` become `.optional()`), for schema/consumer parity with `postgres-kit`. Always compiled; there is no cargo feature gating it. Mirrors the retired TS package's `createSelectSchema`/`createInsertSchema` output style: `camelCase` keys, 4-space formatting, and the same ClickHouse→TS/Zod type mapping (`String`/`UUID`/dates→`string`/`z.string()`, ints/floats→`number`/`z.number()`, `Bool`→`boolean`, `Array(String)`→`string[]`/`z.array(z.string())`, `Map(String,String)`→`Record<string,string>`/`z.record(z.string(),z.string())`, `JSON`→`unknown`/`z.unknown()`, `Nullable(T)`→optional `T | null`/`.nullable()`, `LowCardinality(T)` transparent → `T`).

### Patch Changes

- 94b19c4: Docs/metadata: neutralized external-toolkit references in the ROADMAP, package description/keywords, and source comments (now described as a schema-as-code toolkit with Zod schema emitters). No API change.

## 0.2.0

### Minor Changes

- 0cac5bd: v0.2 safety core — safe-by-construction primitives for user-defined / multi-tenant schemas (ROADMAP item 2). When column names + types come from untrusted input, these enforce the boundary so SQL injection and unbounded tables are impossible on the happy path:
  - `validateIdentifier(name, kind?)` — strict ASCII allowlist + length bound; rejects dots, quotes, backticks, metacharacters, leading digits, unicode, injection attempts.
  - `quoteIdentifier(name)` — backtick-quoting with escape (defense-in-depth).
  - `columnFromTypeSpec(spec)` — builds a `ChColumn` from a JSON-friendly recursive type spec, enforcing an **allowlist** (`String`/ints/floats/`Date`/`DateTime64`/`Bool`/`UUID`/`JSON` + `nullable`/`lowCardinality`/`Array(String)`/`Map(String,String)`); rejects `Decimal`/`FixedString`/`Tuple`/`Enum`/`Nested`/arbitrary type strings. The single gate from outside input to a column.
  - `assertColumnCount` / `assertNotReserved` / `DEFAULT_LIMITS` / `DEFAULT_RESERVED_COLUMNS` — bounds + reserved-column (`attrs`/`raw`) handling.

  Foundation for the runtime table construction + `flexibleTable` primitives in the rest of v0.2.

- 0f48410: v0.2 — the safe foundation for flexible, user-driven, multi-tenant schemas (ROADMAP items 1, 3, 4, 5; item 2 safety core shipped separately).
  - **Runtime table construction**: `clickhouseTableFromSpec(name, columns[], options)` builds a `ChTable` from an untrusted runtime column list (validates identifiers, enforces the type allowlist + column bounds + dedupe), with `runtimeSelectSchema(table)` for a zod validator. Same `toCreateTableSql` rendering as the static path.
  - **Semi-structured columns + hybrid table**: `ch.map()`, `ch.array(inner)`, and `flexibleTable(name, { mandatory, promoted, options })` — the proven mandatory + `attrs Map(String,String)` + `raw String` + promoted-typed-columns shape, with reserved-column guards.
  - **Flatten + coerce**: `flattenRecord(obj, opts?)` (nested → dotted-key string map, arrays stringified, depth/key caps) and `coerceToTable(input, table)` (route known keys to columns, the long tail into the `attrs` catch-all, capture `raw`, report `overflowKeys`).
  - **Additive, bounded evolution**: `diffColumns(table, live)` (additive-only: kit-but-not-live columns) and `alterAddColumnsSql(table, missing)` (guarded `ALTER TABLE … ADD COLUMN IF NOT EXISTS …`, identifiers backtick-quoted, types from the trusted kit definition) — for growing dynamic per-tenant tables without touching the forward-only file migrations.

  All additive, TS-only, safe by construction; built on the v0.2 safety core.

## 0.1.1

### Patch Changes

- fc2dec5: Add `ch.nullable(inner)` (renders `Nullable(<inner>)`, composes with `lowCardinality` for `LowCardinality(Nullable(String))`) and `ch.json()` (the native ClickHouse `JSON` type) column helpers.
