// @smooai/clickhouse-kit — flatten + coerce.
//
// The layer where arbitrary, user-driven data meets a (possibly dynamic) table
// schema. `flattenRecord` turns a nested object into a bounded dotted-key string
// map (the shape a `Map(String, String)` catch-all wants); `coerceToTable`
// shapes an arbitrary record to a table's columns — known keys pass through to
// their column, the long tail is flattened into the catch-all, and `raw` keeps
// the original. Pure, dependency-free, and never invents column names or SQL.

import type { ChTable } from "./kit";

// ── flattenRecord ────────────────────────────────────────────────────────────

export interface FlattenOptions {
  /** Max nesting depth to recurse before stringifying the remaining subtree. */
  maxDepth?: number;
  /** Hard cap on emitted keys — flattening stops once reached (never exceeded). */
  maxKeys?: number;
  /** Separator between nested keys (e.g. `a.b.c`). */
  delimiter?: string;
}

const FLATTEN_DEFAULTS = { maxDepth: 8, maxKeys: 1024, delimiter: "." } as const;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Stringify a leaf value: strings pass through; arrays/objects JSON-stringify; everything else `String()`. */
function stringifyLeaf(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value) || isPlainObject(value)) return JSON.stringify(value);
  return String(value);
}

/**
 * Flatten a nested object into a dotted-key → string map. Nested objects are
 * recursed (`{a:{b:1}}` → `{"a.b":"1"}`); arrays are JSON-stringified rather
 * than recursed; primitives are stringified. Depth and key caps are enforced by
 * stopping (objects past `maxDepth` are JSON-stringified whole; keys past
 * `maxKeys` are skipped) — never by throwing, and never exceeding `maxKeys`.
 */
export function flattenRecord(
  obj: Record<string, unknown>,
  opts?: FlattenOptions,
): Record<string, string> {
  const maxDepth = opts?.maxDepth ?? FLATTEN_DEFAULTS.maxDepth;
  const maxKeys = opts?.maxKeys ?? FLATTEN_DEFAULTS.maxKeys;
  const delimiter = opts?.delimiter ?? FLATTEN_DEFAULTS.delimiter;
  const out: Record<string, string> = {};

  const visit = (node: Record<string, unknown>, prefix: string, depth: number): void => {
    for (const [key, value] of Object.entries(node)) {
      if (Object.keys(out).length >= maxKeys) return;
      if (value === undefined) continue;
      const path = prefix ? `${prefix}${delimiter}${key}` : key;
      if (isPlainObject(value) && depth < maxDepth) {
        visit(value, path, depth + 1);
      } else {
        out[path] = stringifyLeaf(value);
      }
    }
  };

  visit(obj, "", 0);
  return out;
}

// ── coerceToTable ────────────────────────────────────────────────────────────

export interface CoerceResult {
  /** A plain row object shaped to the table's columns — downstream insert/zod validates it. */
  row: Record<string, unknown>;
  /** Input keys that did not match a typed column and were routed to the catch-all. */
  overflowKeys: string[];
}

/**
 * Shape an arbitrary input record to a table's columns. Each input key that
 * matches a (non-reserved) column name passes its value through to that column.
 * Every unmatched key is routed into the catch-all column (default `attrs` when
 * the table has it) as a flattened string map, and recorded in `overflowKeys`.
 * If the table has a `raw` String column, it is set to `JSON.stringify(input)`.
 *
 * The catch-all and `raw` columns are reserved — input keys literally named
 * `attrs`/`raw` are treated as overflow rather than clobbering the managed
 * column. The coercer never validates value types (the zod schema's job) and
 * never invents column names.
 */
export function coerceToTable(
  input: Record<string, unknown>,
  table: ChTable,
  opts?: { catchAll?: string },
): CoerceResult {
  const catchAll = opts?.catchAll ?? "attrs";
  const columns = table.columns;
  const hasCatchAll = Object.prototype.hasOwnProperty.call(columns, catchAll);
  const hasRaw = Object.prototype.hasOwnProperty.call(columns, "raw");

  const row: Record<string, unknown> = {};
  const overflow: Record<string, unknown> = {};
  const overflowKeys: string[] = [];

  for (const [key, value] of Object.entries(input)) {
    const isReserved = key === catchAll || key === "raw";
    if (!isReserved && Object.prototype.hasOwnProperty.call(columns, key)) {
      row[key] = value;
    } else {
      overflow[key] = value;
      overflowKeys.push(key);
    }
  }

  if (hasCatchAll) row[catchAll] = flattenRecord(overflow);
  if (hasRaw) row.raw = JSON.stringify(input);

  return { row, overflowKeys };
}
