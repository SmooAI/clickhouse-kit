// @smooai/clickhouse-kit — runtime table construction.
//
// The static `clickhouseTable(...)` builds a typed table from a developer-authored
// literal. This is its runtime sibling: it builds a `ChTable` from a column list
// produced at RUNTIME (a customer config, a DB row, parsed JSON) — where neither
// the column names nor their types are known at compile time, so there is no static
// type inference to give up. The catch is that the input is untrusted, so every
// spec passes through the `./safety` boundary (identifier validation + the type
// allowlist) before it can reach the DDL. The resulting table shares the exact same
// `toCreateTableSql` rendering and yields a runtime zod validator — safe by
// construction, not merely by convention.

import {
  type ChColumn,
  type ChColumns,
  type ChTable,
  type ChTableOptions,
  createSelectSchema,
} from "./kit";
import {
  assertColumnCount,
  type ColumnTypeSpec,
  columnFromTypeSpec,
  SchemaSafetyError,
  validateIdentifier,
} from "./safety";

/**
 * A single column, as supplied by untrusted runtime input. `type` is the
 * JSON-friendly `ColumnTypeSpec` (gated by the `./safety` allowlist); `default`
 * is an optional ClickHouse DEFAULT expression (e.g. `now()`).
 */
export interface ColumnSpec {
  readonly name: string;
  readonly type: ColumnTypeSpec;
  readonly default?: string;
}

/**
 * Build a `ChTable` from a runtime-supplied column list — the runtime sibling of
 * `clickhouseTable`. Validates the table name, enforces the column-count bound,
 * and for each spec validates the column name, rejects duplicates, and maps the
 * type through the `columnFromTypeSpec` allowlist (the only path from outside input
 * to a column type — there is no arbitrary type string). Returns a generic
 * `ChTable` (no static row inference) whose `toCreateTableSql` works unchanged.
 */
export function clickhouseTableFromSpec(
  name: string,
  columns: ColumnSpec[],
  options: ChTableOptions,
): ChTable {
  validateIdentifier(name, "table");
  assertColumnCount(columns.length);

  const built: Record<string, ChColumn> = {};
  for (const spec of columns) {
    const columnName = validateIdentifier(spec.name, "column");
    if (Object.prototype.hasOwnProperty.call(built, columnName))
      throw new SchemaSafetyError(`duplicate column name ${JSON.stringify(columnName)}`);
    const column = columnFromTypeSpec(spec.type);
    built[columnName] = spec.default ? column.default(spec.default) : column;
  }

  return {
    name,
    columns: built as ChColumns,
    options,
    $inferSelect: undefined as unknown as ChTable["$inferSelect"],
  };
}

/**
 * A runtime zod validator for a row read from a table built via
 * `clickhouseTableFromSpec` — the runtime counterpart to `createSelectSchema`, so
 * callers get a validator matching the DDL the kit just generated.
 */
export function runtimeSelectSchema(table: ChTable) {
  return createSelectSchema(table);
}
