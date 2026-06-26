import { describe, expect, it } from "vitest";
import { z } from "zod";
import {
  createInsertSchema,
  createSelectSchema,
  toCreateMaterializedViewSql,
  toCreateTableSql,
} from "../kit";
import { events, eventsByDayMv, selectEventSchema, validEventRow } from "./fixtures";

describe("clickhouseTable → DDL", () => {
  const ddl = toCreateTableSql(events);

  it("renders columns incl. LowCardinality / Map / DateTime64 / UUID", () => {
    expect(ddl).toContain("CREATE TABLE IF NOT EXISTS events (");
    expect(ddl).toContain("ts DateTime64(3)");
    expect(ddl).toContain("org_id LowCardinality(String)");
    expect(ddl).toContain("event_id UUID");
    expect(ddl).toContain("attributes Map(String, String)");
    expect(ddl).toContain("count UInt16");
  });

  it("renders a column DEFAULT", () => {
    expect(ddl).toContain("ingested_at DateTime DEFAULT now()");
  });

  it("renders engine / partition / order / skip-index / settings", () => {
    expect(ddl).toContain("ENGINE = MergeTree()");
    expect(ddl).toContain("PARTITION BY (org_id, toDate(ts))");
    expect(ddl).toContain("ORDER BY (org_id, ts, event_id)");
    expect(ddl).toContain("INDEX idx_name name TYPE bloom_filter(0.01) GRANULARITY 1");
    expect(ddl).toContain("storage_policy = 'hot_cold'");
    expect(ddl).toContain("index_granularity = 8192");
  });

  it("AUTO-WRAPS the DateTime64 TTL column in toDateTime()", () => {
    // A `TTL ... TO VOLUME` move on a DateTime64 column must render as
    // toDateTime(...), NOT the raw column (which throws BAD_TTL_EXPRESSION).
    expect(ddl).toContain("TTL toDateTime(ts) + INTERVAL 14 DAY TO VOLUME 'cold'");
    expect(ddl).toContain("toDateTime(ts) + INTERVAL 90 DAY DELETE");
    expect(ddl).not.toMatch(/TTL ts \+ INTERVAL/);
  });

  it("throws when TTL references an unknown column", () => {
    expect(() =>
      toCreateTableSql({
        ...events,
        options: { ...events.options, ttl: { column: "nope", deleteAfter: "1 DAY" } },
      }),
    ).toThrow(/unknown column/);
  });
});

describe("materialized view → DDL", () => {
  it("renders CREATE MATERIALIZED VIEW ... TO ... AS <select>", () => {
    const ddl = toCreateMaterializedViewSql(eventsByDayMv);
    expect(ddl).toContain("CREATE MATERIALIZED VIEW IF NOT EXISTS events_by_day_mv");
    expect(ddl).toContain("TO events_by_day AS");
    expect(ddl).toContain("SELECT org_id, toDate(ts) AS day");
  });
});

describe("drizzle-zod ergonomics", () => {
  it("createSelectSchema parses a row and rejects a wrong type", () => {
    expect(selectEventSchema.safeParse(validEventRow).success).toBe(true);
    expect(selectEventSchema.safeParse({ ...validEventRow, value: "nope" }).success).toBe(false);
  });

  it("createInsertSchema makes DEFAULT columns optional, others required", () => {
    const insert = createInsertSchema(events);
    const { ingested_at, ...withoutDefault } = validEventRow;
    expect(insert.safeParse(withoutDefault).success).toBe(true);
    const { name, ...withoutName } = withoutDefault;
    expect(insert.safeParse(withoutName).success).toBe(false);
  });

  it("honors per-column overrides", () => {
    const refined = createSelectSchema(events, { kind: z.enum(["server", "client"]) });
    expect(refined.safeParse(validEventRow).success).toBe(true);
    expect(refined.safeParse({ ...validEventRow, kind: "WAT" }).success).toBe(false);
  });
});
