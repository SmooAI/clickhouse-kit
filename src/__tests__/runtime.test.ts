import { describe, expect, it } from "vitest";
import { type ChTableOptions, toCreateTableSql } from "../kit";
import { clickhouseTableFromSpec, type ColumnSpec, runtimeSelectSchema } from "../runtime";
import { DEFAULT_LIMITS, SchemaSafetyError } from "../safety";

// A representative runtime column list — the kind a customer config / DB row / JSON
// would yield: scalars, a 64-bit int (read as string), the allowed wrappers, a Map.
const specs: ColumnSpec[] = [
  { name: "org_id", type: { lowCardinality: "String" } },
  { name: "event_id", type: "UUID" },
  { name: "ts", type: "DateTime64" },
  { name: "value", type: "Float64" },
  { name: "big", type: "UInt64" },
  { name: "tags", type: { array: "String" } },
  { name: "attributes", type: { map: ["String", "String"] } },
  { name: "note", type: { nullable: "String" } },
];

const options: ChTableOptions = {
  engine: "MergeTree()",
  orderBy: ["org_id", "ts"],
};

describe("clickhouseTableFromSpec → DDL", () => {
  it("builds a table from runtime specs and renders the expected DDL", () => {
    const table = clickhouseTableFromSpec("events", specs, options);
    const ddl = toCreateTableSql(table);
    expect(ddl).toContain("CREATE TABLE IF NOT EXISTS events (");
    expect(ddl).toContain("org_id LowCardinality(String)");
    expect(ddl).toContain("event_id UUID");
    expect(ddl).toContain("ts DateTime64(3)");
    expect(ddl).toContain("value Float64");
    expect(ddl).toContain("big UInt64");
    expect(ddl).toContain("tags Array(String)");
    expect(ddl).toContain("attributes Map(String, String)");
    expect(ddl).toContain("note Nullable(String)");
    expect(ddl).toContain("ENGINE = MergeTree()");
    expect(ddl).toContain("ORDER BY (org_id, ts)");
  });

  it("applies a column DEFAULT from the spec", () => {
    const table = clickhouseTableFromSpec(
      "with_default",
      [
        { name: "id", type: "String" },
        { name: "ingested_at", type: "DateTime", default: "now()" },
      ],
      options,
    );
    expect(toCreateTableSql(table)).toContain("ingested_at DateTime DEFAULT now()");
  });
});

describe("clickhouseTableFromSpec — safe by construction", () => {
  it("rejects duplicate column names", () => {
    expect(() =>
      clickhouseTableFromSpec(
        "dup",
        [
          { name: "a", type: "String" },
          { name: "a", type: "Int32" },
        ],
        options,
      ),
    ).toThrow(SchemaSafetyError);
  });

  it("rejects an invalid column name (no SQL injection reaches the DDL)", () => {
    expect(() =>
      clickhouseTableFromSpec("evil", [{ name: "a; DROP TABLE x", type: "String" }], options),
    ).toThrow(SchemaSafetyError);
  });

  it("rejects an invalid table name", () => {
    expect(() =>
      clickhouseTableFromSpec("bad name", [{ name: "a", type: "String" }], options),
    ).toThrow(SchemaSafetyError);
  });

  it("rejects a disallowed column type", () => {
    expect(() =>
      clickhouseTableFromSpec(
        "t",
        // `Decimal(38,10)` is not on the allowlist — must never reach the DDL.
        [{ name: "a", type: "Decimal(38,10)" as unknown as ColumnSpec["type"] }],
        options,
      ),
    ).toThrow(SchemaSafetyError);
  });

  it("rejects more than maxColumns columns", () => {
    const tooMany: ColumnSpec[] = Array.from(
      { length: DEFAULT_LIMITS.maxColumns + 1 },
      (_unused, i) => ({ name: `c${i}`, type: "String" }),
    );
    expect(() => clickhouseTableFromSpec("big", tooMany, options)).toThrow(SchemaSafetyError);
  });

  it("rejects an empty column list", () => {
    expect(() => clickhouseTableFromSpec("empty", [], options)).toThrow(SchemaSafetyError);
  });
});

describe("runtimeSelectSchema", () => {
  it("parses a valid row and rejects a wrong type", () => {
    const table = clickhouseTableFromSpec(
      "rows",
      [
        { name: "org_id", type: { lowCardinality: "String" } },
        { name: "value", type: "Float64" },
        { name: "note", type: { nullable: "String" } },
      ],
      options,
    );
    const schema = runtimeSelectSchema(table);
    expect(schema.safeParse({ org_id: "org_1", value: 1.5, note: null }).success).toBe(true);
    expect(schema.safeParse({ org_id: "org_1", value: "nope", note: "x" }).success).toBe(false);
  });
});
