// @smooai/clickhouse-kit — public API.
//
// "Drizzle for ClickHouse": define a table once, get the DDL, the inferred row
// type, and drizzle-zod-style select/insert schemas. Forward-only migrations
// (no auto-diff engine) that ride your own ClickHouse client + a drift gate.

export {
  ch,
  ChColumn,
  type ChColumns,
  type ChIndex,
  type ChMaterializedView,
  type ChTable,
  type ChTableOptions,
  type ChTtl,
  clickhouseMaterializedView,
  clickhouseTable,
  createInsertSchema,
  createSelectSchema,
  type InferSelect,
  toCreateMaterializedViewSql,
  toCreateTableSql,
} from "./kit";

export {
  type AppliedMigration,
  CH_MIGRATIONS_TABLE,
  type ClickHouseClient,
  defaultMigrationsDir,
  listMigrationFiles,
  type MigrationRunResult,
  runClickHouseMigrations,
  splitSqlStatements,
} from "./migrate";

export {
  generateClickHouseMigrations,
  type GenerateResult,
  type Journal,
  type JournalEntry,
  journalPath,
  loadJournal,
  migrationFilename,
  nextMigrationNumber,
} from "./generate";

export {
  checkClickHouseDrift,
  type Drift,
  type DriftKind,
  type DriftResult,
  expectedColumns,
  type LiveColumn,
} from "./check";

// Safe-by-construction primitives for user-defined / multi-tenant schemas.
export {
  ALLOWED_SCALAR_TYPES,
  type AllowedScalarType,
  assertColumnCount,
  assertNotReserved,
  columnFromTypeSpec,
  type ColumnTypeSpec,
  DEFAULT_LIMITS,
  DEFAULT_RESERVED_COLUMNS,
  quoteIdentifier,
  type SchemaLimits,
  SchemaSafetyError,
  validateIdentifier,
} from "./safety";

// Runtime (user-defined) table construction.
export { clickhouseTableFromSpec, type ColumnSpec, runtimeSelectSchema } from "./runtime";

// Semi-structured / catch-all hybrid table.
export { flexibleTable, type FlexibleTableConfig } from "./flexible";

// Flatten + coerce arbitrary input to a (possibly dynamic) table's columns.
export { type CoerceResult, coerceToTable, type FlattenOptions, flattenRecord } from "./flatten";

// Additive, bounded schema evolution (guarded ALTER ADD COLUMN).
// (evolve.ts's LiveColumn mirrors ./check's — re-use that one to avoid a duplicate export.)
export { alterAddColumnsSql, type ColumnDiff, diffColumns } from "./evolve";
