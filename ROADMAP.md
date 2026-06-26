# Roadmap

## v0.1 (shipped) — static, developer-authored schemas

"Drizzle for ClickHouse": a developer authors a table once, at compile time, as a literal — `clickhouseTable(name, columns, options)` → `toCreateTableSql` (DDL) + inferred row type (`InferSelect`) + `createSelectSchema`/`createInsertSchema` (drizzle-zod) + forward-only migrations (`generate`/`migrate`/`check`, no auto-diff). Column system: `ch.*` (`ChColumn`). Minimal, TS-only, forward-only, MIT.

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

## Next: Rust-canonical (the customer-shape work runs in Rust)

The flexible / multi-tenant surface (runtime construction, the safety layer, flatten/coerce, additive evolution) is consumed by **Rust** services (api-prime, audit-logs, the Ask-Your-Data data platform) — that's where untrusted customer schema input is turned into SQL, and _safe-by-construction only counts in the process holding the input_. So the canonical implementation moves to Rust:

- **Canonical Rust crate** (`crates/clickhouse-kit`, crates.io, MIT) — rows are Serde-native (`#[derive(Row)]` via the `clickhouse` crate); the crate adds the allowlisted type system, identifier safety, DDL generation, runtime/`flexible` construction, additive evolution, migrations, and drift. The allowlist is _stronger_ than the TS version: disallowed types are **unrepresentable** (no enum variant), so untrusted JSON naming them fails to deserialize at the boundary.
- **No TS consumers → the Rust crate is the standalone product.** Published as **`smooai-clickhouse-kit`** on crates.io (imports as `clickhouse_kit`). There is nothing on the TS side that needs to ride this, so there is **no WASM/npm binding**; the Rust services consume the crate directly. The original TS package served as the reference spec and is retired/static-only.
- **The TS v0.1/v0.2 is the reference spec** the Rust port mirrors (the adversarial safety tests translate almost line-for-line).
- **Tradeoff recorded:** TS compile-time `$inferSelect` row inference can't come from a WASM/runtime core — static TS-authored tables move to codegen'd types. Non-issue for dynamic tables (shapes unknown at compile time).

Started: the Rust **safety core** (`crates/clickhouse-kit/src/safety.rs`) — `validate_identifier`/`quote_identifier`, the `ColumnTypeSpec` allowlist (+ `to_ch_type`/`is_datetime64`), bounds + reserved — plus runtime **table DDL generation** (`table.rs`: `to_create_table_sql` from an untrusted spec, with identifier/allowlist/bounds/dup guards). Verified **end-to-end against a real ClickHouse** via testcontainers (generate DDL → apply → introspect `system.columns` → insert/select round-trip); the ported adversarial unit suite (injection, disallowed types, bounds, dup columns) is green too. CI runs unit + the testcontainers integration.

**Full surface landed (built via a 4-way Rust fan-out, lead-integrated):** `flexible_table` (the hybrid), `flatten_record` + `coerce_to_table`, `diff_columns` + `alter_add_columns_sql` (additive-only), and the I/O layer — a driver-agnostic `ChExecutor` trait + `run_migrations` (forward-only) + `check_drift` — with a second testcontainers integration exercising migrate + drift against a real ClickHouse. **38 unit + 2 real-ClickHouse integration tests, clippy `-D warnings` clean.** Published to crates.io as **`smooai-clickhouse-kit`** (manual `publish-crate.yml`, `SMOOAI_CARGO_REGISTRY_TOKEN`). No WASM binding — there are no TS consumers; the Rust services consume the crate directly. Rows stay Serde-native.
