# Roadmap

## v0.1 (shipped) — static, developer-authored schemas

A schema-as-code toolkit for ClickHouse: a developer authors a table once, at compile time, as a literal — `clickhouseTable(name, columns, options)` → `toCreateTableSql` (DDL) + inferred row type (`InferSelect`) + `createSelectSchema`/`createInsertSchema` (Zod schema emitters) + forward-only migrations (`generate`/`migrate`/`check`, no auto-diff). Column system: `ch.*` (`ChColumn`). Minimal, TS-only, forward-only, MIT.

## v0.2 — the safe foundation for flexible, user-driven, multi-tenant schemas

**Reframe:** compile-time (developer) and runtime (end-user/customer) schemas look opposed but need the _same primitives_ — a bounded type system, DDL generation, identifier safety, row validation, additive evolution. v0.1 has the first two for the static case. v0.2 exposes those primitives through a **runtime-constructable path alongside** the static literal one, without losing static type inference.

**Positioning:** the kit becomes the toolkit for building flexible, multi-tenant, user-defined ClickHouse schemas **safely by construction** — not a static DDL helper. Apps stop hand-rolling allowed-type allowlists, identifier sanitization, flatten-to-Map catch-alls, schemaless DDL, and ad-hoc `ALTER ADD COLUMN`; those are general ClickHouse-multitenancy concerns and belong here.

**Status:** items 1–5 shipped in **0.2.0**; item 6 is partially shipped (`ch.json()` is available; the explicit ≥24.8 version-gating docs are still to come).

### Capabilities (all additive; preserve the kit's character)

1. **Runtime table construction** — `clickhouseTableFromSpec(name, columnSpec[], options)`: accepts a runtime-built column list (from a customer config / DB row / JSON), returns a generic `ChTable` with the same `toCreateTableSql` + a runtime zod validator. Static `clickhouseTable` stays for typed dev tables; share the rendering underneath.
2. **Safety primitives (untrusted input)** — exported + enforced: an **allowed-type allowlist** (`String`, `Nullable(String)`, ints/floats, `Date`, `DateTime64`, `Bool`, `LowCardinality(...)`, `Map(String,String)`, `Array(String)`, `JSON` — reject `Decimal`/`FixedString`/`Tuple`/arbitrary exprs); **identifier validation + backtick-quoting** for table/column names (no SQL injection); **reserved-column** handling; **bounds** (max columns/table, max identifier length).
3. **Semi-structured / catch-all primitives** — `ch.nullable(inner)` ✅, `ch.json()` ✅ (shipped in 0.1.1), plus `ch.map()`, `ch.array(inner)`; and a **`flexibleTable()`** helper encoding the proven hybrid: mandatory cols + `attrs Map(String,String)` + `raw String` + caller-supplied promoted typed columns. The single most reused pattern → a one-liner.
4. **Flatten + coerce** — `flattenRecord(obj)` (nested → dotted-key string map; arrays stringified; depth/key caps) + a coercer that shapes an arbitrary record to a (possibly dynamic) table's columns, routing the long tail into `attrs`/`JSON`. The kit owns validation/coercion so it matches the DDL it generated.
5. **Additive, bounded evolution** — `diffColumns(table, liveColumns)` + an **additive-only** `alterAddColumnsSql(table, missing)` (new columns only; optional type-widening). Complements (does not replace) forward-only file migrations: dynamic per-tenant tables evolve via this guarded ALTER path; static dev tables stay on numbered migrations. `check`/drift keeps working per dynamic table.
6. **Native JSON, version-gated** — `ch.json()` is the long-term answer for nested/variable shapes (typed `data.foo` paths). Documented CH ≥24.8 floor; `Map(String,String)` + raw stays the works-everywhere fallback.

### Invariants (don't drift)

- **Minimal + additive.** No ORM, no query builder, no auto-diff engine for static tables (the additive ALTER is a separate, explicitly-bounded path).
- **Forward-only** migrations remain the model for code-defined tables.
- **MIT, generic.** Frame every primitive as "multi-tenant ClickHouse," never coupled to one app.
- **Safe by construction.** Every runtime/user-facing primitive validates input; the happy path makes SQL injection and unbounded tables impossible, not merely discouraged.

## Source-of-truth model: TypeScript for static, Rust for dynamic (split by population)

ClickHouse has two schema populations with different natural owners. This mirrors `smooai-postgres-kit`'s TS-source reframe for the developer-authored set, while keeping the runtime engine canonical where TypeScript can't reach:

- **Static, developer-authored tables** (observability, metrics, billing) → **TypeScript is the source of truth** (the `@smooai/clickhouse-kit` TS authoring DX). The Rust crate is the **TS→Rust bridge**: `introspect` reads the live ClickHouse → Rust, `codegen` (`rust_row_struct` / `ch_type_to_rust`) emits the `#[derive(Row)]` struct, and `check_drift` asserts the Rust view ≡ the live (TS-owned) schema — so the Rust side never hand-copies or silently diverges.
- **Dynamic, customer-defined / multi-tenant tables** (Ask-Your-Data, custom tables, audit) → **Rust is canonical**. Created at runtime from untrusted input; safe-by-construction only counts in the process holding the input. The allowlisted type system, identifier safety, DDL gen, `flexible_table`, forward-only migrations, and additive evolution live here. The allowlist is **unrepresentable-by-default** — disallowed types have no enum variant, so untrusted JSON naming them fails to deserialize at the boundary.
- **Crate:** `smooai-clickhouse-kit` on crates.io (imports as `clickhouse_kit`); rows stay Serde-native. **No WASM/npm binding** — the TS side authors static schemas in its own kit; the Rust side bridges + owns the dynamic engine.

Started: the Rust **safety core** (`crates/clickhouse-kit/src/safety.rs`) — `validate_identifier`/`quote_identifier`, the `ColumnTypeSpec` allowlist (+ `to_ch_type`/`is_datetime64`), bounds + reserved — plus runtime **table DDL generation** (`table.rs`: `to_create_table_sql` from an untrusted spec, with identifier/allowlist/bounds/dup guards). Verified **end-to-end against a real ClickHouse** via testcontainers (generate DDL → apply → introspect `system.columns` → insert/select round-trip); the ported adversarial unit suite (injection, disallowed types, bounds, dup columns) is green too. CI runs unit + the testcontainers integration.

**Full surface landed:** the runtime toolkit — `flexible_table`, `flatten_record`/`coerce_to_table`, `diff_columns`/`alter_add_columns_sql` (additive-only), the driver-agnostic `ChExecutor` trait + `run_migrations` (forward-only) + `check_drift` — plus **production-table DDL** on `TableSpec` (`partition_by`/`ttl`/`indexes`/`settings`, parametrized `DateTime64(p, tz)`; SMOODEV-2115). And **codegen both directions**: `ch_type_to_rust`/`rust_row_struct` + `introspect_row_struct` (live ClickHouse → Rust `#[derive(Row)]` — the TS→Rust bridge), and `emit_row_interface`/`emit_select_schema`/`emit_insert_schema`/`emit_ts_module` (a `TableSpec` → TS interface + Zod select/insert schemas). Verified against real ClickHouse via three testcontainers integrations (DDL round-trip; migrate + drift; introspect → codegen); clippy `-D warnings` clean. Published to crates.io as **`smooai-clickhouse-kit` 0.3.0** (manual `publish-crate.yml`, `SMOOAI_CARGO_REGISTRY_TOKEN`). Rows stay Serde-native.
