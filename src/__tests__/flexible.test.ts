import { describe, expect, it } from "vitest";
import { ch, toCreateTableSql } from "../kit";
import { flexibleTable } from "../flexible";

describe("ch.map / ch.array", () => {
  it("ch.map() renders Map(String, String)", () => {
    expect(ch.map().chType).toBe("Map(String, String)");
  });

  it("ch.array(ch.string()) renders Array(String)", () => {
    expect(ch.array(ch.string()).chType).toBe("Array(String)");
  });

  it("ch.array composes the inner column's type", () => {
    expect(ch.array(ch.uint32()).chType).toBe("Array(UInt32)");
  });

  it("ch.map() value zod parses a string map and rejects a non-string value", () => {
    expect(ch.map().zodType.safeParse({ a: "b" }).success).toBe(true);
    expect(ch.map().zodType.safeParse({ a: 1 }).success).toBe(false);
  });

  it("ch.array() value zod parses an array and rejects a wrong element type", () => {
    expect(ch.array(ch.string()).zodType.safeParse(["a", "b"]).success).toBe(true);
    expect(ch.array(ch.string()).zodType.safeParse(["a", 1]).success).toBe(false);
  });
});

describe("flexibleTable", () => {
  const t = flexibleTable("events", {
    mandatory: { org_id: ch.lowCardinality(ch.string()), ts: ch.dateTime64(3) },
    promoted: { source: ch.lowCardinality(ch.string()), count: ch.uint32() },
    options: { engine: "MergeTree()", orderBy: ["org_id", "ts"] },
  });
  const ddl = toCreateTableSql(t);

  it("renders the mandatory + promoted columns + attrs Map + raw String", () => {
    expect(ddl).toContain("CREATE TABLE IF NOT EXISTS events (");
    expect(ddl).toContain("org_id LowCardinality(String)");
    expect(ddl).toContain("ts DateTime64(3)");
    expect(ddl).toContain("source LowCardinality(String)");
    expect(ddl).toContain("count UInt32");
    expect(ddl).toContain("attrs Map(String, String)");
    expect(ddl).toContain("raw String");
  });

  it("adds attrs + raw even with no mandatory/promoted columns", () => {
    const bare = flexibleTable("bare", { options: { engine: "MergeTree()", orderBy: ["raw"] } });
    const d = toCreateTableSql(bare);
    expect(d).toContain("attrs Map(String, String)");
    expect(d).toContain("raw String");
  });

  it("throws when a promoted column collides with a reserved name (attrs)", () => {
    expect(() =>
      flexibleTable("bad", {
        promoted: { attrs: ch.string() },
        options: { engine: "MergeTree()", orderBy: ["raw"] },
      }),
    ).toThrow(/reserved/);
  });

  it("throws when a mandatory column collides with a reserved name (raw)", () => {
    expect(() =>
      flexibleTable("bad", {
        mandatory: { raw: ch.string() },
        options: { engine: "MergeTree()", orderBy: ["raw"] },
      }),
    ).toThrow(/reserved/);
  });

  it("throws on an invalid table name", () => {
    expect(() =>
      flexibleTable("bad name!", { options: { engine: "MergeTree()", orderBy: ["raw"] } }),
    ).toThrow();
  });

  it("throws on an invalid column name", () => {
    expect(() =>
      flexibleTable("ok", {
        promoted: { "bad-col": ch.string() },
        options: { engine: "MergeTree()", orderBy: ["raw"] },
      }),
    ).toThrow();
  });

  it("honors a custom reserved list", () => {
    expect(() =>
      flexibleTable("custom", {
        promoted: { tenant: ch.string() },
        options: { engine: "MergeTree()", orderBy: ["raw"] },
        reserved: ["attrs", "raw", "tenant"],
      }),
    ).toThrow(/reserved/);
  });
});
