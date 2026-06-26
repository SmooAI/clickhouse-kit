//! Forward-only migration runner. Applies `*.sql` files from a directory in
//! lexical order, recording each in a `_ch_migrations` bookkeeping table so
//! re-runs are idempotent. There is no auto-diff and no down-migration — schema
//! change is expressed as ordered, append-only SQL files.

use crate::client::{ChError, ChExecutor};
use std::path::Path;

/// The bookkeeping table that records which migration files have been applied.
const MIGRATIONS_TABLE: &str = "_ch_migrations";

/// Outcome of a [`run_migrations`] pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRunResult {
    /// Every `*.sql` file found in the directory, lexically sorted.
    pub discovered: Vec<String>,
    /// Files already recorded as applied (skipped this pass).
    pub skipped: Vec<String>,
    /// Files applied during this pass, in the order they ran.
    pub applied: Vec<String>,
}

/// Ensure the bookkeeping table exists, then apply every pending `*.sql` file in
/// `dir` (lexical order), recording each as it succeeds. Idempotent: files
/// already present in `_ch_migrations` are skipped.
pub async fn run_migrations(
    exec: &impl ChExecutor,
    dir: &Path,
) -> Result<MigrationRunResult, ChError> {
    ensure_migrations_table(exec).await?;

    let discovered = discover_migration_files(dir)?;
    let already_applied = fetch_applied(exec).await?;

    let mut skipped = Vec::new();
    let mut applied = Vec::new();

    for filename in &discovered {
        if already_applied.contains(filename) {
            skipped.push(filename.clone());
            continue;
        }

        let path = dir.join(filename);
        let sql = std::fs::read_to_string(&path)?;
        for statement in split_sql_statements(&sql) {
            exec.command(&statement).await?;
        }
        exec.command(&record_statement(filename)).await?;
        applied.push(filename.clone());
    }

    Ok(MigrationRunResult {
        discovered,
        skipped,
        applied,
    })
}

/// Create the `_ch_migrations` table if it does not already exist.
async fn ensure_migrations_table(exec: &impl ChExecutor) -> Result<(), ChError> {
    let ddl = format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (\n\
         \x20   filename String,\n\
         \x20   applied_at DateTime DEFAULT now()\n\
         )\nENGINE = MergeTree\nORDER BY filename"
    );
    exec.command(&ddl).await
}

/// Read the already-applied migration filenames from the bookkeeping table.
async fn fetch_applied(exec: &impl ChExecutor) -> Result<Vec<String>, ChError> {
    exec.fetch_strings(&format!(
        "SELECT filename FROM {MIGRATIONS_TABLE} ORDER BY filename"
    ))
    .await
}

/// Record a single applied migration filename.
fn record_statement(filename: &str) -> String {
    // Filenames come from a trusted directory listing, but escape single quotes
    // defensively so an odd filename can't break the INSERT.
    let escaped = filename.replace('\'', "''");
    format!("INSERT INTO {MIGRATIONS_TABLE} (filename) VALUES ('{escaped}')")
}

/// List `*.sql` files in `dir`, returning their filenames in lexical order.
fn discover_migration_files(dir: &Path) -> Result<Vec<String>, ChError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sql") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                files.push(name.to_string());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Split a migration file into individual statements: strip `--` line comments,
/// split on `;`, and drop empty fragments.
pub fn split_sql_statements(sql: &str) -> Vec<String> {
    let stripped: String = sql
        .lines()
        .map(|line| match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");

    stripped
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_strips_comments() {
        let sql = "-- create the table\n\
                   CREATE TABLE x (a Int32) ENGINE = Memory; -- trailing\n\
                   INSERT INTO x VALUES (1);\n\
                   \n\
                   -- a whole-line comment\n";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "CREATE TABLE x (a Int32) ENGINE = Memory");
        assert_eq!(stmts[1], "INSERT INTO x VALUES (1)");
    }

    #[test]
    fn empty_input_yields_no_statements() {
        assert!(split_sql_statements("").is_empty());
        assert!(split_sql_statements("   \n  ;; \n -- only a comment").is_empty());
    }

    #[test]
    fn record_statement_escapes_quotes() {
        assert_eq!(
            record_statement("001_init.sql"),
            "INSERT INTO _ch_migrations (filename) VALUES ('001_init.sql')"
        );
        assert_eq!(
            record_statement("o'brien.sql"),
            "INSERT INTO _ch_migrations (filename) VALUES ('o''brien.sql')"
        );
    }
}
