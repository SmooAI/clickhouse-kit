// @smooai/clickhouse-kit — TS-only, Drizzle-shaped schema kit for ClickHouse.
//
// Define a table ONCE with `clickhouseTable(...)` and get: the `CREATE TABLE` DDL
// (`toCreateTableSql`), the inferred TS row type (`typeof table.$inferSelect`),
// and select/insert Zod (`createSelectSchema`/`createInsertSchema`) — mirroring
// the Drizzle + drizzle-zod ergonomics. Forward-only by design: there is NO schema
// differ (schema changes are hand-authored migrations, like Drizzle custom SQL).

import { z } from "zod";

// ── Columns ────────────────────────────────────────────────────────────────
// A column carries its ClickHouse SQL type, the Zod type for its value, whether
// it's a DateTime64 (so TTL expressions auto-wrap in toDateTime — see below),
// and an optional DEFAULT expression. `T` is the inferred TS type of the value.
export class ChColumn<T = unknown> {
  constructor(
    readonly chType: string,
    readonly zodType: z.ZodType<T>,
    readonly isDateTime64 = false,
    readonly defaultExpr?: string,
  ) {}

  /** Add a ClickHouse DEFAULT expression (e.g. `now()`). Makes the column optional on insert. */
  default(expr: string): ChColumn<T> {
    return new ChColumn(this.chType, this.zodType, this.isDateTime64, expr);
  }
}

export const ch = {
  string: () => new ChColumn<string>("String", z.string()),
  uuid: () => new ChColumn<string>("UUID", z.uuid()),
  float64: () => new ChColumn<number>("Float64", z.number()),
  uint8: () => new ChColumn<number>("UInt8", z.number().int()),
  uint16: () => new ChColumn<number>("UInt16", z.number().int()),
  uint32: () => new ChColumn<number>("UInt32", z.number().int()),
  uint64: () => new ChColumn<string>("UInt64", z.string()), // JS-safe: read as string
  /**
   * DateTime64(precision[, tz]) — read over the HTTP interface as an ISO-ish string.
   * Pass `tz` (e.g. 'UTC') to render `DateTime64(3, 'UTC')`. Still flagged isDateTime64
   * so TTL expressions on this column auto-wrap in toDateTime().
   */
  dateTime64: (precision = 3, tz?: string) =>
    new ChColumn<string>(`DateTime64(${precision}${tz ? `, '${tz}'` : ""})`, z.string(), true),
  dateTime: () => new ChColumn<string>("DateTime", z.string()),
  /** LowCardinality wrapper around an inner column type (keeps the inner's Zod). */
  lowCardinality: <T>(inner: ChColumn<T>) =>
    new ChColumn<T>(`LowCardinality(${inner.chType})`, inner.zodType, inner.isDateTime64),
  /**
   * Nullable wrapper around an inner column type — `Nullable(<inner>)`, with a
   * nullable Zod. Composes with lowCardinality for the common
   * `LowCardinality(Nullable(String))`: `ch.lowCardinality(ch.nullable(ch.string()))`.
   */
  nullable: <T>(inner: ChColumn<T>) =>
    new ChColumn<T | null>(
      `Nullable(${inner.chType})`,
      inner.zodType.nullable(),
      inner.isDateTime64,
    ),
  mapStringString: () =>
    new ChColumn<Record<string, string>>("Map(String, String)", z.record(z.string(), z.string())),
  /** A JSON column (the native ClickHouse `JSON` type) — read as an object. */
  json: () => new ChColumn<Record<string, unknown>>("JSON", z.record(z.string(), z.unknown())),
  /**
   * An AggregateFunction state column (AggregatingMergeTree material), e.g.
   * `ch.aggregateFunction('quantilesTDigest(0.5, 0.95, 0.99), Float64')` →
   * `AggregateFunction(quantilesTDigest(0.5, 0.95, 0.99), Float64)`. The on-disk
   * value is an opaque aggregation state — selected raw it's not a meaningful JS
   * value (you `-Merge()` it in a query), so the default Zod is `z.unknown()`;
   * pass `valueZod` to refine if a consumer reads a finalized form.
   */
  aggregateFunction: <T = unknown>(signature: string, valueZod?: z.ZodType<T>) =>
    new ChColumn<T>(`AggregateFunction(${signature})`, (valueZod ?? z.unknown()) as z.ZodType<T>),
};

// ── Table ────────────────────────────────────────────────────────────────────
export interface ChIndex {
  readonly name: string;
  readonly expr: string;
  readonly type: string; // e.g. "bloom_filter(0.01)" | "tokenbf_v1(32768, 3, 0)"
  readonly granularity: number;
}

export interface ChTtl {
  /** Column the TTL is computed from. Auto-wrapped in toDateTime() if it's a DateTime64. */
  readonly column: string;
  /** Move partitions to a storage volume after this interval (e.g. "14 DAY"). */
  readonly moveToVolumeAfter?: { readonly interval: string; readonly volume: string };
  /** Delete partitions after this interval (e.g. "180 DAY"). */
  readonly deleteAfter?: string;
}

export interface ChTableOptions {
  readonly engine: string; // e.g. "MergeTree()"
  readonly orderBy: readonly string[];
  readonly partitionBy?: string;
  readonly ttl?: ChTtl;
  readonly indexes?: readonly ChIndex[];
  readonly settings?: Readonly<Record<string, string | number>>;
}

export type ChColumns = Record<string, ChColumn>;
export type InferSelect<C extends ChColumns> = {
  [K in keyof C]: C[K] extends ChColumn<infer T> ? T : never;
};

export interface ChTable<C extends ChColumns = ChColumns> {
  readonly name: string;
  readonly columns: C;
  readonly options: ChTableOptions;
  /** Phantom — `typeof table.$inferSelect` is the row type. Never read at runtime. */
  readonly $inferSelect: InferSelect<C>;
}

export function clickhouseTable<C extends ChColumns>(
  name: string,
  columns: C,
  options: ChTableOptions,
): ChTable<C> {
  return { name, columns, options, $inferSelect: undefined as unknown as InferSelect<C> };
}

// ── DDL generation ─────────────────────────────────────────────────────────
function renderTtl(ttl: ChTtl, columns: ChColumns): string {
  // A `TTL ... TO VOLUME` move requires a DateTime/Date result, but timestamp
  // columns are often DateTime64 — so wrap them in toDateTime() automatically.
  // Makes the ClickHouse BAD_TTL_EXPRESSION footgun structurally impossible.
  const col = columns[ttl.column];
  if (!col) throw new Error(`TTL references unknown column "${ttl.column}"`);
  const expr = col.isDateTime64 ? `toDateTime(${ttl.column})` : ttl.column;
  const clauses: string[] = [];
  if (ttl.moveToVolumeAfter)
    clauses.push(
      `${expr} + INTERVAL ${ttl.moveToVolumeAfter.interval} TO VOLUME '${ttl.moveToVolumeAfter.volume}'`,
    );
  if (ttl.deleteAfter) clauses.push(`${expr} + INTERVAL ${ttl.deleteAfter} DELETE`);
  return clauses.join(",\n    ");
}

/** Render the `CREATE TABLE IF NOT EXISTS` DDL for a table definition. */
export function toCreateTableSql<C extends ChColumns>(table: ChTable<C>): string {
  const colLines = Object.entries(table.columns).map(
    ([name, col]) =>
      `    ${name} ${col.chType}${col.defaultExpr ? ` DEFAULT ${col.defaultExpr}` : ""}`,
  );
  const indexLines = (table.options.indexes ?? []).map(
    (i) => `    INDEX ${i.name} ${i.expr} TYPE ${i.type} GRANULARITY ${i.granularity}`,
  );
  const body = [...colLines, ...indexLines].join(",\n");

  const parts = [
    `CREATE TABLE IF NOT EXISTS ${table.name} (`,
    body,
    `)`,
    `ENGINE = ${table.options.engine}`,
  ];
  if (table.options.partitionBy) parts.push(`PARTITION BY ${table.options.partitionBy}`);
  parts.push(`ORDER BY (${table.options.orderBy.join(", ")})`);
  if (table.options.ttl) parts.push(`TTL ${renderTtl(table.options.ttl, table.columns)}`);
  if (table.options.settings) {
    const settings = Object.entries(table.options.settings)
      .map(([k, v]) => `${k} = ${typeof v === "string" ? `'${v}'` : v}`)
      .join(", ");
    parts.push(`SETTINGS ${settings}`);
  }
  return parts.join("\n");
}

// ── Zod (drizzle-zod ergonomics) ─────────────────────────────────────────────
type ShapeOf<C extends ChColumns> = {
  [K in keyof C]: C[K] extends ChColumn<infer T> ? z.ZodType<T> : never;
};

/** Zod schema for a row as read from ClickHouse. Pass `overrides` to refine columns (like drizzle-zod). */
export function createSelectSchema<C extends ChColumns>(
  table: ChTable<C>,
  overrides?: Partial<Record<keyof C, z.ZodTypeAny>>,
): z.ZodObject<ShapeOf<C>> {
  const shape: Record<string, z.ZodTypeAny> = {};
  for (const [name, col] of Object.entries(table.columns))
    shape[name] = overrides?.[name as keyof C] ?? col.zodType;
  return z.object(shape) as unknown as z.ZodObject<ShapeOf<C>>;
}

/** Zod schema for an insert row — columns with a DEFAULT become optional. */
export function createInsertSchema<C extends ChColumns>(
  table: ChTable<C>,
  overrides?: Partial<Record<keyof C, z.ZodTypeAny>>,
): z.ZodObject<z.ZodRawShape> {
  const shape: Record<string, z.ZodTypeAny> = {};
  for (const [name, col] of Object.entries(table.columns)) {
    const base = overrides?.[name as keyof C] ?? col.zodType;
    shape[name] = col.defaultExpr ? base.optional() : base;
  }
  return z.object(shape);
}

// ── Materialized views ───────────────────────────────────────────────────────
// A minimal `CREATE MATERIALIZED VIEW ... TO <target> AS <select>` builder. The
// kit deliberately does NOT model the SELECT body (no column inference, no aggregate
// state typing) — the SELECT is a raw string and the target table is the typed
// source of truth for what the MV writes.
export interface ChMaterializedView {
  readonly name: string;
  /** The target table the MV writes into (`TO <to>`). */
  readonly to: string;
  /** The raw SELECT body (everything after `AS`). */
  readonly asSelect: string;
}

export function clickhouseMaterializedView(
  name: string,
  options: { to: string; asSelect: string },
): ChMaterializedView {
  return { name, to: options.to, asSelect: options.asSelect };
}

/** Render the `CREATE MATERIALIZED VIEW IF NOT EXISTS ... TO ... AS <select>` DDL. */
export function toCreateMaterializedViewSql(mv: ChMaterializedView): string {
  return [
    `CREATE MATERIALIZED VIEW IF NOT EXISTS ${mv.name}`,
    `TO ${mv.to} AS`,
    mv.asSelect.trim(),
  ].join("\n");
}
