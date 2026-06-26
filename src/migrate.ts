// @smooai/clickhouse-kit — forward-only ClickHouse migration runner.
//
// Deliberately forward-only: there is NO auto-diff engine. Migrations are
// hand-authored / generator-emitted `.sql` files, applied in lexical order, each
// recorded in a `_ch_migrations` tracking table so already-applied files are
// skipped on the next run.
//
// Bring your own client: `runClickHouseMigrations(client, dir)` takes any object
// matching the small `ClickHouseClient` structural interface (the official
// `@clickhouse/client` satisfies it), so this package has ZERO runtime client
// dependency. You own connection + credential management.

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

/**
 * The structural subset of a ClickHouse client the runner uses. The official
 * `@clickhouse/client` `ClickHouseClient` satisfies this — pass it directly.
 */
export interface ClickHouseClient {
  command(args: { query: string }): Promise<unknown>;
  query(args: { query: string; query_params?: Record<string, unknown>; format?: string }): Promise<{
    json<T>(): Promise<T[]> | T[] | Promise<unknown>;
  }>;
  close?(): Promise<void>;
}

/** Name of the table that tracks which migrations have been applied. */
export const CH_MIGRATIONS_TABLE = "_ch_migrations";

const CREATE_MIGRATIONS_TABLE_DDL = `CREATE TABLE IF NOT EXISTS ${CH_MIGRATIONS_TABLE} (
    filename String,
    applied_at DateTime DEFAULT now()
)
ENGINE = MergeTree()
ORDER BY filename`;

export interface AppliedMigration {
  filename: string;
}

export interface MigrationRunResult {
  /** Filenames found in the migrations dir, in lexical order. */
  discovered: string[];
  /** Filenames already recorded as applied (skipped this run). */
  skipped: string[];
  /** Filenames applied during this run, in order. */
  applied: string[];
}

/** Default migrations directory: `<cwd>/clickhouse/migrations`. Override per call. */
export function defaultMigrationsDir(): string {
  return path.resolve(process.cwd(), "clickhouse", "migrations");
}

/**
 * Split a `.sql` file into individual statements on `;` boundaries. ClickHouse's
 * HTTP interface runs one statement per call, so multi-statement files must be
 * split. Line comments (`-- …`) are stripped so a trailing-comment-only fragment
 * doesn't become an empty statement.
 */
export function splitSqlStatements(sql: string): string[] {
  return sql
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Read the forward-only `.sql` migration files in lexical order. */
export function listMigrationFiles(migrationsDir: string): string[] {
  return readdirSync(migrationsDir)
    .filter((f) => f.endsWith(".sql"))
    .sort((a, b) => a.localeCompare(b));
}

async function ensureMigrationsTable(client: ClickHouseClient): Promise<void> {
  await client.command({ query: CREATE_MIGRATIONS_TABLE_DDL });
}

async function fetchAppliedFilenames(client: ClickHouseClient): Promise<Set<string>> {
  const result = await client.query({
    query: `SELECT filename FROM ${CH_MIGRATIONS_TABLE} ORDER BY filename`,
    format: "JSONEachRow",
  });
  const rows = (await result.json<AppliedMigration>()) as AppliedMigration[];
  return new Set(rows.map((r) => r.filename));
}

async function recordApplied(client: ClickHouseClient, filename: string): Promise<void> {
  // filename is repo-controlled (a file on disk), not user input — single-quote
  // escape defensively all the same.
  const escaped = filename.replace(/'/g, "\\'");
  await client.command({
    query: `INSERT INTO ${CH_MIGRATIONS_TABLE} (filename) VALUES ('${escaped}')`,
  });
}

/**
 * Apply pending ClickHouse migrations.
 *
 * Forward-only: ensures the tracking table exists, reads `.sql` files in lexical
 * order, skips any already recorded in `_ch_migrations`, and applies the rest
 * (splitting multi-statement files and running each via `client.command`). Each
 * applied file is recorded after its statements succeed, so a failure mid-file
 * leaves it un-recorded and it is retried on the next run.
 *
 * Pure with respect to connection management — the caller owns the client.
 */
export async function runClickHouseMigrations(
  client: ClickHouseClient,
  migrationsDir: string = defaultMigrationsDir(),
): Promise<MigrationRunResult> {
  await ensureMigrationsTable(client);
  const applied = await fetchAppliedFilenames(client);

  const discovered = listMigrationFiles(migrationsDir);
  const result: MigrationRunResult = { discovered, skipped: [], applied: [] };

  for (const filename of discovered) {
    if (applied.has(filename)) {
      result.skipped.push(filename);
      continue;
    }
    const sql = readFileSync(path.join(migrationsDir, filename), "utf-8");
    for (const statement of splitSqlStatements(sql)) {
      await client.command({ query: statement });
    }
    await recordApplied(client, filename);
    result.applied.push(filename);
  }

  return result;
}
