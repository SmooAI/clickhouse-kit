// @smooai/clickhouse-kit — forward-only migration generator.
//
// Given your kit-defined tables + materialized views, emit a new numbered
// migration `.sql` file (and append it to `_journal.json`) containing the
// `CREATE` DDL for any object not yet captured. There is NO auto-diff engine:
// this only ever appends a brand-new CREATE for a not-yet-migrated table/MV.
// Schema *changes* to an already-migrated object are hand-authored as a fresh
// migration, exactly like Drizzle custom migrations.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import {
  type ChMaterializedView,
  type ChTable,
  toCreateMaterializedViewSql,
  toCreateTableSql,
} from "./kit";
import { defaultMigrationsDir, listMigrationFiles } from "./migrate";

export interface JournalEntry {
  idx: number;
  filename: string;
  /** Schema-object name (table or MV) this migration creates. The dedup key. */
  table: string;
}

export interface Journal {
  version: string;
  dialect: "clickhouse";
  entries: JournalEntry[];
}

const JOURNAL_FILE = "_journal.json";

export function journalPath(migrationsDir: string): string {
  return path.join(migrationsDir, JOURNAL_FILE);
}

export function loadJournal(migrationsDir: string): Journal {
  const p = journalPath(migrationsDir);
  if (!existsSync(p)) return { version: "1", dialect: "clickhouse", entries: [] };
  return JSON.parse(readFileSync(p, "utf-8")) as Journal;
}

function writeJournal(migrationsDir: string, journal: Journal): void {
  writeFileSync(journalPath(migrationsDir), `${JSON.stringify(journal, null, 4)}\n`, "utf-8");
}

/** Zero-padded 4-digit migration number prefix, e.g. 0001, 0002. */
export function nextMigrationNumber(existingFiles: string[]): number {
  let max = 0;
  for (const f of existingFiles) {
    const m = /^(\d{4})_/.exec(f);
    if (m) max = Math.max(max, Number(m[1]));
  }
  return max + 1;
}

export function migrationFilename(num: number, objectName: string): string {
  return `${String(num).padStart(4, "0")}_${objectName}.sql`;
}

export interface GenerateResult {
  /** Object names that already had a captured migration (no-op). */
  skipped: string[];
  /** Newly written migration files, in order. */
  created: { filename: string; table: string }[];
}

/**
 * Emit migrations for any table/MV whose CREATE DDL isn't yet captured by the
 * journal. Tables are emitted before materialized views, so a MV (which reads/
 * writes tables that must already exist) always lands at a higher migration
 * number than its dependencies. Pure with respect to the migrations dir.
 */
export function generateClickHouseMigrations(
  migrationsDir: string = defaultMigrationsDir(),
  tables: readonly ChTable[] = [],
  materializedViews: readonly ChMaterializedView[] = [],
): GenerateResult {
  if (!existsSync(migrationsDir)) mkdirSync(migrationsDir, { recursive: true });

  const journal = loadJournal(migrationsDir);
  const captured = new Set(journal.entries.map((e) => e.table));
  const existingFiles = listMigrationFiles(migrationsDir);

  const result: GenerateResult = { skipped: [], created: [] };
  let nextNum = nextMigrationNumber(existingFiles);

  const emit = (name: string, ddl: string): void => {
    if (captured.has(name)) {
      result.skipped.push(name);
      return;
    }
    const filename = migrationFilename(nextNum, name);
    writeFileSync(path.join(migrationsDir, filename), `${ddl};\n`, "utf-8");
    journal.entries.push({ idx: journal.entries.length, filename, table: name });
    captured.add(name);
    result.created.push({ filename, table: name });
    nextNum += 1;
  };

  for (const table of tables) emit(table.name, toCreateTableSql(table));
  for (const mv of materializedViews) emit(mv.name, toCreateMaterializedViewSql(mv));

  if (result.created.length > 0) writeJournal(migrationsDir, journal);
  return result;
}
