import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { type ClickHouseClient, runClickHouseMigrations, splitSqlStatements } from "../migrate";

// An in-memory fake ClickHouse client: records DDL statements run via command()
// and tracks the `_ch_migrations` rows so SELECT reflects what's been applied.
function fakeClient() {
  const applied: string[] = [];
  const statements: string[] = [];
  const client: ClickHouseClient = {
    command: async ({ query }) => {
      statements.push(query);
      const m = /INSERT INTO _ch_migrations \(filename\) VALUES \('(.+)'\)/.exec(query);
      if (m) applied.push(m[1]!);
      return undefined;
    },
    query: async () => ({ json: async () => applied.map((filename) => ({ filename })) }),
  };
  return { client, applied, statements };
}

describe("splitSqlStatements", () => {
  it("splits on ; and strips line comments", () => {
    expect(
      splitSqlStatements(
        "-- a comment\nCREATE TABLE a (x String) ENGINE=Memory;\nINSERT INTO a VALUES (1);",
      ),
    ).toEqual(["CREATE TABLE a (x String) ENGINE=Memory", "INSERT INTO a VALUES (1)"]);
  });

  it("ignores trailing-only and empty fragments", () => {
    expect(splitSqlStatements("SELECT 1;\n   \n;")).toEqual(["SELECT 1"]);
  });
});

describe("runClickHouseMigrations", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(path.join(tmpdir(), "smooai-ch-kit-"));
    writeFileSync(path.join(dir, "0001_a.sql"), "CREATE TABLE a (x String) ENGINE = Memory;\n");
    writeFileSync(path.join(dir, "0002_b.sql"), "CREATE TABLE b (y String) ENGINE = Memory;\n");
  });
  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  it("applies all pending migrations in order, recording each", async () => {
    const { client, applied } = fakeClient();
    const result = await runClickHouseMigrations(client, dir);
    expect(result.discovered).toEqual(["0001_a.sql", "0002_b.sql"]);
    expect(result.applied).toEqual(["0001_a.sql", "0002_b.sql"]);
    expect(result.skipped).toEqual([]);
    expect(applied).toEqual(["0001_a.sql", "0002_b.sql"]);
  });

  it("skips already-applied migrations on a second run (idempotent)", async () => {
    const { client } = fakeClient();
    await runClickHouseMigrations(client, dir);
    const second = await runClickHouseMigrations(client, dir);
    expect(second.applied).toEqual([]);
    expect(second.skipped).toEqual(["0001_a.sql", "0002_b.sql"]);
  });

  it("applies only the new migration when one is added", async () => {
    const { client } = fakeClient();
    await runClickHouseMigrations(client, dir);
    writeFileSync(path.join(dir, "0003_c.sql"), "CREATE TABLE c (z String) ENGINE = Memory;\n");
    const result = await runClickHouseMigrations(client, dir);
    expect(result.applied).toEqual(["0003_c.sql"]);
    expect(result.skipped).toEqual(["0001_a.sql", "0002_b.sql"]);
  });
});
