import { describe, expect, it } from "vitest";
import { toCreateTableSql } from "../kit";
import {
  ALLOWED_SCALAR_TYPES,
  assertColumnCount,
  assertNotReserved,
  columnFromTypeSpec,
  DEFAULT_LIMITS,
  quoteIdentifier,
  SchemaSafetyError,
  validateIdentifier,
} from "../safety";

describe("validateIdentifier", () => {
  it("accepts safe identifiers", () => {
    for (const ok of ["a", "A", "_x", "org_id", "col1", "X_2_y"])
      expect(validateIdentifier(ok)).toBe(ok);
  });

  it("rejects SQL-injection / metacharacter attempts", () => {
    const attacks = [
      "a; DROP TABLE x",
      "a`,`b",
      "a) ENGINE=Memory AS SELECT * FROM secrets --",
      "a' OR '1'='1",
      "a b",
      "a.b",
      "a-b",
      "1col",
      "",
      'a"b',
      "a\nb",
      "таблица",
      "a/*x*/",
    ];
    for (const bad of attacks)
      expect(() => validateIdentifier(bad), bad).toThrow(SchemaSafetyError);
  });

  it("enforces the length bound", () => {
    expect(() => validateIdentifier("a".repeat(DEFAULT_LIMITS.maxIdentifierLength + 1))).toThrow(
      /too long/,
    );
    expect(validateIdentifier("a".repeat(DEFAULT_LIMITS.maxIdentifierLength))).toBeTruthy();
  });
});

describe("quoteIdentifier", () => {
  it("backtick-wraps and escapes embedded backticks (defense-in-depth)", () => {
    expect(quoteIdentifier("org_id")).toBe("`org_id`");
    expect(quoteIdentifier("a`b")).toBe("`a``b`");
  });
});

describe("assertColumnCount", () => {
  it("rejects empty and over-limit", () => {
    expect(() => assertColumnCount(0)).toThrow(/at least one/);
    expect(() => assertColumnCount(DEFAULT_LIMITS.maxColumns + 1)).toThrow(/too many/);
    expect(() => assertColumnCount(10)).not.toThrow();
  });
});

describe("assertNotReserved", () => {
  it("rejects reserved names", () => {
    expect(() => assertNotReserved("attrs")).toThrow(SchemaSafetyError);
    expect(() => assertNotReserved("raw")).toThrow(SchemaSafetyError);
    expect(() => assertNotReserved("user_col")).not.toThrow();
  });
});

describe("columnFromTypeSpec — allowlist", () => {
  it("builds every allowed scalar", () => {
    for (const t of ALLOWED_SCALAR_TYPES)
      expect(columnFromTypeSpec(t).chType).toContain(t === "DateTime64" ? "DateTime64" : t);
  });

  it("builds the allowed wrappers/containers", () => {
    expect(columnFromTypeSpec({ nullable: "String" }).chType).toBe("Nullable(String)");
    expect(columnFromTypeSpec({ lowCardinality: { nullable: "String" } }).chType).toBe(
      "LowCardinality(Nullable(String))",
    );
    expect(columnFromTypeSpec({ array: "String" }).chType).toBe("Array(String)");
    expect(columnFromTypeSpec({ map: ["String", "String"] }).chType).toBe("Map(String, String)");
  });

  it("preserves the DateTime64 TTL-wrap flag through Nullable/LowCardinality", () => {
    expect(columnFromTypeSpec({ lowCardinality: "DateTime64" }).isDateTime64).toBe(true);
  });

  it("REJECTS disallowed types (no arbitrary type string reaches the DDL)", () => {
    const bad: unknown[] = [
      "Decimal(38, 10)",
      "FixedString(16)",
      "Enum8('a' = 1)",
      "Tuple(String, Int32)",
      "Nested(x String)",
      "String) ENGINE=Memory AS SELECT 1 --",
      { map: ["String", "Int32"] },
      { array: "Int32" },
      { array: { nullable: "String" } },
      { wat: "String" },
      42,
      null,
    ];
    for (const b of bad)
      expect(() => columnFromTypeSpec(b as never), JSON.stringify(b)).toThrow(SchemaSafetyError);
  });

  it("the built column renders into safe DDL", () => {
    const t = {
      name: "evt",
      columns: {
        ts: columnFromTypeSpec("DateTime64"),
        tags: columnFromTypeSpec({ array: "String" }),
      },
      options: { engine: "MergeTree()", orderBy: ["ts"] },
      $inferSelect: undefined as never,
    };
    const ddl = toCreateTableSql(t);
    expect(ddl).toContain("ts DateTime64(3)");
    expect(ddl).toContain("tags Array(String)");
  });
});
