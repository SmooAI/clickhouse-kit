// @smooai/clickhouse-kit — schema drift gate.
//
// Compares the live ClickHouse schema (system.columns) against your kit-defined
// tables and reports drift: a table that doesn't exist, a column present in the
// kit but not the DB (or vice versa), or a type mismatch. Forward-only
// philosophy: this DETECTS drift; it never auto-repairs.
//
// Scope: plain tables only. Materialized views derive their columns from a SELECT
// and aren't column-checked here (their existence is covered by migrations).

import type { ClickHouseClient } from "./migrate";
import type { ChTable } from "./kit";

export interface LiveColumn {
  name: string;
  type: string;
}

export type DriftKind = "missing_table" | "missing_column" | "extra_column" | "type_mismatch";

export interface Drift {
  table: string;
  kind: DriftKind;
  column?: string;
  expected?: string;
  actual?: string;
}

export interface DriftResult {
  checked: string[];
  drift: Drift[];
}

/** The columns the kit expects for a table: name → ClickHouse type. */
export function expectedColumns(table: ChTable): Map<string, string> {
  return new Map(Object.entries(table.columns).map(([name, col]) => [name, col.chType]));
}

// ClickHouse canonicalizes types in system.columns (spacing etc.). Normalize both
// sides before comparing so cosmetic differences ("Map(String, String)" vs
// "Map(String,String)") don't read as drift.
function normalizeType(t: string): string {
  return t.replace(/\s+/g, "");
}

async function fetchLiveColumns(
  client: ClickHouseClient,
  tableName: string,
): Promise<LiveColumn[]> {
  const result = await client.query({
    query: `SELECT name, type FROM system.columns WHERE database = currentDatabase() AND table = {t:String} ORDER BY position`,
    query_params: { t: tableName },
    format: "JSONEachRow",
  });
  return (await result.json<LiveColumn>()) as LiveColumn[];
}

/** Compare each kit table against the live ClickHouse schema. */
export async function checkClickHouseDrift(
  client: ClickHouseClient,
  tables: readonly ChTable[],
): Promise<DriftResult> {
  const result: DriftResult = { checked: [], drift: [] };

  for (const table of tables) {
    result.checked.push(table.name);
    const live = await fetchLiveColumns(client, table.name);

    if (live.length === 0) {
      result.drift.push({ table: table.name, kind: "missing_table" });
      continue;
    }

    const expected = expectedColumns(table);
    const liveByName = new Map(live.map((c) => [c.name, c.type]));

    for (const [name, expectedType] of expected) {
      const actualType = liveByName.get(name);
      if (actualType === undefined) {
        result.drift.push({
          table: table.name,
          kind: "missing_column",
          column: name,
          expected: expectedType,
        });
      } else if (normalizeType(actualType) !== normalizeType(expectedType)) {
        result.drift.push({
          table: table.name,
          kind: "type_mismatch",
          column: name,
          expected: expectedType,
          actual: actualType,
        });
      }
    }
    for (const { name, type } of live) {
      if (!expected.has(name))
        result.drift.push({ table: table.name, kind: "extra_column", column: name, actual: type });
    }
  }

  return result;
}
