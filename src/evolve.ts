// @smooai/clickhouse-kit — additive, bounded schema evolution.
//
// Dynamic, per-tenant tables (built at runtime from a customer config / DB row /
// JSON) need to grow as new attributes get promoted to typed columns. This is the
// GUARDED additive-only ALTER path: it diffs a kit table against the live schema
// and emits `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` for columns the kit declares
// but the DB lacks. It COMPLEMENTS — does not replace — the forward-only file
// migrations: static, code-defined tables stay on numbered migrations; this is the
// explicitly-bounded escape hatch for runtime-defined ones.
//
// Additive ONLY, by construction: no DROP, no MODIFY/retype. Reporting extra or
// retyped columns is the drift gate's job (see check.ts) — this path only adds.
// The added column's TYPE always comes from the trusted, kit-generated table
// definition, never from the (untrusted) live schema, and identifiers are
// backtick-quoted via the safety primitives as defense-in-depth.

import type { ChTable } from "./kit";
import { quoteIdentifier } from "./safety";

/** A column as introspected from the live ClickHouse schema (mirrors check.ts). */
export interface LiveColumn {
  name: string;
  type: string;
}

/** A column the kit declares that is absent from the live schema. */
export interface ColumnDiff {
  name: string;
  expectedType: string;
}

/**
 * Columns present in the kit `table` but absent from `live` (compared by name).
 * ADDITIVE ONLY: live-only columns and type differences are deliberately ignored
 * — surfacing those is the drift gate's responsibility (`checkClickHouseDrift`).
 */
export function diffColumns(table: ChTable, live: LiveColumn[]): { missing: ColumnDiff[] } {
  const liveNames = new Set(live.map((c) => c.name));
  const missing: ColumnDiff[] = [];
  for (const [name, col] of Object.entries(table.columns)) {
    if (!liveNames.has(name)) missing.push({ name, expectedType: col.chType });
  }
  return { missing };
}

/**
 * One `ALTER TABLE <table> ADD COLUMN IF NOT EXISTS <col> <type>` per missing
 * column. The table + column identifiers are backtick-quoted (defense-in-depth);
 * the column TYPE is read from the kit table's own `ChColumn.chType` (trusted,
 * kit-generated) — NEVER from untrusted live data. Returns `[]` when in sync.
 */
export function alterAddColumnsSql(table: ChTable, missing: ColumnDiff[]): string[] {
  return missing.map(({ name }) => {
    const col = table.columns[name];
    if (!col) throw new Error(`cannot add column "${name}": not defined on table "${table.name}"`);
    return `ALTER TABLE ${quoteIdentifier(table.name)} ADD COLUMN IF NOT EXISTS ${quoteIdentifier(name)} ${col.chType}`;
  });
}
