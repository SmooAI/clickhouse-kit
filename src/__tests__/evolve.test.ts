import { describe, expect, it } from "vitest";
import { alterAddColumnsSql, type ColumnDiff, diffColumns, type LiveColumn } from "../evolve";
import { ch, clickhouseTable } from "../kit";

// A small inline runtime-shaped table: a couple of mandatory typed columns plus
// some that a tenant has "promoted" over time (kind, value).
const tenant = clickhouseTable(
  "tenant_events",
  {
    org_id: ch.string(),
    ts: ch.dateTime64(3),
    kind: ch.lowCardinality(ch.string()),
    value: ch.float64(),
  },
  { engine: "MergeTree()", orderBy: ["org_id", "ts"] },
);

/** Live schema as if the DB only has the two original columns. */
const liveOriginal: LiveColumn[] = [
  { name: "org_id", type: "String" },
  { name: "ts", type: "DateTime64(3)" },
];

describe("diffColumns", () => {
  it("finds only columns present in the kit but absent from live", () => {
    const { missing } = diffColumns(tenant, liveOriginal);
    expect(missing.map((m) => m.name)).toEqual(["kind", "value"]);
  });

  it("reports the kit chType as the expected type", () => {
    const { missing } = diffColumns(tenant, liveOriginal);
    expect(missing).toEqual<ColumnDiff[]>([
      { name: "kind", expectedType: "LowCardinality(String)" },
      { name: "value", expectedType: "Float64" },
    ]);
  });

  it("ignores live-only columns (additive only — never reports them)", () => {
    const liveWithExtra: LiveColumn[] = [
      ...liveOriginal,
      { name: "kind", type: "LowCardinality(String)" },
      { name: "value", type: "Float64" },
      // A column the DB has that the kit does NOT declare — must be ignored here.
      { name: "legacy_blob", type: "String" },
    ];
    const { missing } = diffColumns(tenant, liveWithExtra);
    expect(missing).toEqual([]);
  });

  it("returns nothing when fully in sync", () => {
    const liveFull: LiveColumn[] = [
      { name: "org_id", type: "String" },
      { name: "ts", type: "DateTime64(3)" },
      { name: "kind", type: "LowCardinality(String)" },
      { name: "value", type: "Float64" },
    ];
    expect(diffColumns(tenant, liveFull).missing).toEqual([]);
  });
});

describe("alterAddColumnsSql", () => {
  it("emits ADD COLUMN IF NOT EXISTS per missing column with the kit's chType", () => {
    const { missing } = diffColumns(tenant, liveOriginal);
    const sql = alterAddColumnsSql(tenant, missing);
    expect(sql).toEqual([
      "ALTER TABLE `tenant_events` ADD COLUMN IF NOT EXISTS `kind` LowCardinality(String)",
      "ALTER TABLE `tenant_events` ADD COLUMN IF NOT EXISTS `value` Float64",
    ]);
  });

  it("backtick-quotes the table and column identifiers", () => {
    const sql = alterAddColumnsSql(tenant, [{ name: "value", expectedType: "Float64" }]);
    expect(sql[0]).toContain("`tenant_events`");
    expect(sql[0]).toContain("`value`");
  });

  it("derives the type from the trusted kit definition, not the supplied diff", () => {
    // Even if a caller hands a bogus expectedType, the emitted DDL uses the kit's chType.
    const sql = alterAddColumnsSql(tenant, [
      { name: "value", expectedType: "String; DROP TABLE x" },
    ]);
    expect(sql).toEqual(["ALTER TABLE `tenant_events` ADD COLUMN IF NOT EXISTS `value` Float64"]);
  });

  it("returns an empty array when nothing is missing", () => {
    expect(alterAddColumnsSql(tenant, [])).toEqual([]);
  });

  it("throws if asked to add a column the table does not define", () => {
    expect(() => alterAddColumnsSql(tenant, [{ name: "ghost", expectedType: "String" }])).toThrow(
      /not defined on table/,
    );
  });
});
