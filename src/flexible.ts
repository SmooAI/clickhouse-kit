// @smooai/clickhouse-kit — the flexible/hybrid table helper.
//
// The single most reused multi-tenant ClickHouse shape, as a one-liner: a few
// mandatory columns the app always controls, an open-ended `attrs Map(String,
// String)` catch-all for the long tail of user-supplied keys, a `raw String`
// holding the untouched original payload, plus any number of caller-supplied
// PROMOTED typed columns (frequently-queried keys lifted out of `attrs` into
// real, indexable columns). Built on `clickhouseTable` underneath, so it shares
// the same DDL rendering, drift checks, and zod ergonomics as a static table.
//
// Safe by construction: the table name and every caller-supplied column name are
// validated against the strict identifier allowlist, and none may collide with
// the reserved catch-all/raw names.

import { ch, clickhouseTable } from "./kit";
import type { ChColumns, ChTable, ChTableOptions } from "./kit";
import { assertNotReserved, DEFAULT_RESERVED_COLUMNS, validateIdentifier } from "./safety";

export interface FlexibleTableConfig {
  /** Columns the app always controls (e.g. org_id, ts). Validated + reserved-checked. */
  readonly mandatory?: ChColumns;
  /** Frequently-queried keys lifted out of `attrs` into real typed columns. Validated + reserved-checked. */
  readonly promoted?: ChColumns;
  /** Engine / order / partition / TTL / indexes / settings, as for `clickhouseTable`. */
  readonly options: ChTableOptions;
  /** Reserved column names callers may not use (defaults to `attrs`, `raw`). */
  readonly reserved?: readonly string[];
}

/**
 * Build the hybrid flexible table: `mandatory` + `promoted` typed columns, an
 * `attrs Map(String, String)` catch-all, and a `raw String`. The name and all
 * caller-supplied column names are identifier-validated and checked against the
 * reserved names; the catch-all (`attrs`) + raw payload (`raw`) are always added.
 */
export function flexibleTable(name: string, config: FlexibleTableConfig): ChTable {
  validateIdentifier(name, "table");
  const reserved = config.reserved ?? DEFAULT_RESERVED_COLUMNS;

  const columns: ChColumns = {};
  for (const [colName, col] of [
    ...Object.entries(config.mandatory ?? {}),
    ...Object.entries(config.promoted ?? {}),
  ]) {
    validateIdentifier(colName, "column");
    assertNotReserved(colName, reserved);
    columns[colName] = col;
  }

  // The hybrid's two always-present columns: the open-ended catch-all + the
  // untouched original payload. Added last so they sort after the typed columns.
  columns.attrs = ch.map();
  columns.raw = ch.string();

  return clickhouseTable(name, columns, config.options);
}
