# @smooai/clickhouse-kit

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
