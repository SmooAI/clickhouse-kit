---
"@smooai/clickhouse-kit": minor
---

v0.2 safety core — safe-by-construction primitives for user-defined / multi-tenant schemas (ROADMAP item 2). When column names + types come from untrusted input, these enforce the boundary so SQL injection and unbounded tables are impossible on the happy path:

- `validateIdentifier(name, kind?)` — strict ASCII allowlist + length bound; rejects dots, quotes, backticks, metacharacters, leading digits, unicode, injection attempts.
- `quoteIdentifier(name)` — backtick-quoting with escape (defense-in-depth).
- `columnFromTypeSpec(spec)` — builds a `ChColumn` from a JSON-friendly recursive type spec, enforcing an **allowlist** (`String`/ints/floats/`Date`/`DateTime64`/`Bool`/`UUID`/`JSON` + `nullable`/`lowCardinality`/`Array(String)`/`Map(String,String)`); rejects `Decimal`/`FixedString`/`Tuple`/`Enum`/`Nested`/arbitrary type strings. The single gate from outside input to a column.
- `assertColumnCount` / `assertNotReserved` / `DEFAULT_LIMITS` / `DEFAULT_RESERVED_COLUMNS` — bounds + reserved-column (`attrs`/`raw`) handling.

Foundation for the runtime table construction + `flexibleTable` primitives in the rest of v0.2.
