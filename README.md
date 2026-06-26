# @smooai/clickhouse-kit

**Drizzle-shaped, TS-only schema toolkit for ClickHouse.** Define a table once and get the `CREATE TABLE` DDL, the inferred TypeScript row type, and [drizzle-zod](https://orm.drizzle.team/docs/zod)-style `select`/`insert` schemas — plus forward-only migrations and a schema-drift gate.

If you like defining a Drizzle `pgTable` and calling `createSelectSchema(table)`, this gives you the same ergonomics for ClickHouse.

```ts
import { ch, clickhouseTable, toCreateTableSql, createSelectSchema } from "@smooai/clickhouse-kit";

export const events = clickhouseTable(
  "events",
  {
    ts: ch.dateTime64(3),
    org_id: ch.lowCardinality(ch.string()),
    event_id: ch.uuid(),
    name: ch.string(),
    value: ch.float64(),
    attributes: ch.mapStringString(),
    ingested_at: ch.dateTime().default("now()"),
  },
  {
    engine: "MergeTree()",
    partitionBy: "(org_id, toDate(ts))",
    orderBy: ["org_id", "ts", "event_id"],
    // Declared structurally — DateTime64 columns auto-wrap in toDateTime()
    // for the TO VOLUME move, so you can't emit a BAD_TTL_EXPRESSION.
    ttl: {
      column: "ts",
      moveToVolumeAfter: { interval: "14 DAY", volume: "cold" },
      deleteAfter: "90 DAY",
    },
    settings: { storage_policy: "hot_cold" },
  },
);

toCreateTableSql(events); // → the CREATE TABLE DDL
createSelectSchema(events); // → a Zod schema for a read row (with optional per-column overrides)
type EventRow = typeof events.$inferSelect; // → the inferred TS row type
```

## Why

The ClickHouse TypeScript ecosystem has no mature, vendor-neutral schema/migration tool with first-class Zod. This is that: schema-as-code, type-safe rows, Zod for ingest/query validation, versioned migrations, and drift detection — in plain TypeScript, with **no bun requirement and no managed-service lock-in**.

## Install

```bash
npm i @smooai/clickhouse-kit zod
# or: pnpm add @smooai/clickhouse-kit zod
```

`zod` (v4) is a peer dependency. The official `@clickhouse/client` (or anything matching the small `ClickHouseClient` interface) is what you pass to the runner/drift gate — this package has **no runtime client dependency**.

## Schema as code

- Columns: `ch.string()`, `ch.uuid()`, `ch.float64()`, `ch.uint8/16/32/64()`, `ch.dateTime()`, `ch.dateTime64(precision, tz?)`, `ch.lowCardinality(inner)`, `ch.mapStringString()`, `ch.aggregateFunction(signature, zod?)`. Add a `DEFAULT` with `.default('now()')`.
- `clickhouseTable(name, columns, options)` — `engine`, `orderBy`, `partitionBy`, `ttl`, `indexes`, `settings`.
- `clickhouseMaterializedView(name, { to, asSelect })` for `CREATE MATERIALIZED VIEW … TO … AS …`.
- `createSelectSchema(table, overrides?)` / `createInsertSchema(table, overrides?)` — Zod, drizzle-zod style. `insert` makes `DEFAULT` columns optional.

## Migrations (forward-only)

Keep your table definitions in a module, then generate numbered `.sql` migrations from them:

```ts
import { generateClickHouseMigrations } from "@smooai/clickhouse-kit";

generateClickHouseMigrations("clickhouse/migrations", [events], [eventsByDayMv]);
// → writes 0001_events.sql, 0002_events_by_day_mv.sql + _journal.json (tables before MVs)
```

Apply them with your own client:

```ts
import { createClient } from "@clickhouse/client";
import { runClickHouseMigrations } from "@smooai/clickhouse-kit";

const client = createClient({ url, username, password });
const result = await runClickHouseMigrations(client, "clickhouse/migrations");
// ensures a _ch_migrations tracking table, applies only un-applied files, records each
```

Wire those two into your own `package.json` scripts (e.g. `db:generate:clickhouse` / `db:migrate`) — the package is library-first and doesn't impose a CLI.

### No auto-diff — on purpose

There is **no schema differ**. New tables get a generated `CREATE`; _changes_ to an existing table are hand-authored as a fresh forward-only migration — exactly like Drizzle's custom SQL migrations. A correct ClickHouse `ALTER` differ (immutable `ORDER BY`, materialized-view recreation, quirky TTL/settings semantics) is a large, brittle surface we deliberately don't ship.

## Drift gate

```ts
import { checkClickHouseDrift } from "@smooai/clickhouse-kit";

const { drift } = await checkClickHouseDrift(client, [events]);
// reports missing_table / missing_column / extra_column / type_mismatch (tolerant of cosmetic type spacing)
if (drift.length) process.exit(1);
```

Run it in CI or post-deploy to catch live schema that has diverged from your definitions.

## License

MIT © [SmooAI](https://smooai.com)
