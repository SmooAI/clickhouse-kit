import { describe, expect, it } from "vitest";
import { coerceToTable, flattenRecord } from "../flatten";
import { ch, clickhouseTable } from "../kit";

describe("flattenRecord", () => {
  it("flattens nested objects into dotted keys", () => {
    const flat = flattenRecord({ a: 1, b: { c: 2, d: { e: "x" } } });
    expect(flat).toEqual({ a: "1", "b.c": "2", "b.d.e": "x" });
  });

  it("JSON-stringifies arrays instead of recursing", () => {
    const flat = flattenRecord({ tags: ["a", "b"], nested: { ids: [1, 2] } });
    expect(flat.tags).toBe('["a","b"]');
    expect(flat["nested.ids"]).toBe("[1,2]");
  });

  it("stringifies primitives and skips undefined", () => {
    const flat = flattenRecord({ n: 3.5, b: true, z: null, u: undefined });
    expect(flat).toEqual({ n: "3.5", b: "true", z: "null" });
    expect("u" in flat).toBe(false);
  });

  it("honors a custom delimiter", () => {
    expect(flattenRecord({ a: { b: 1 } }, { delimiter: "/" })).toEqual({ "a/b": "1" });
  });

  it("enforces the depth cap by JSON-stringifying the remaining subtree", () => {
    // maxDepth 1 → recurse one level, then stringify whatever is left.
    const flat = flattenRecord({ a: { b: { c: 1 } } }, { maxDepth: 1 });
    expect(flat).toEqual({ "a.b": '{"c":1}' });
  });

  it("enforces the key cap without ever exceeding it", () => {
    const flat = flattenRecord({ a: 1, b: 2, c: 3, d: 4 }, { maxKeys: 2 });
    expect(Object.keys(flat).length).toBe(2);
  });
});

// Small flexible-table fixture: mandatory typed cols + attrs catch-all + raw.
const interactions = clickhouseTable(
  "interactions",
  {
    org_id: ch.lowCardinality(ch.string()),
    user_id: ch.string(),
    attrs: ch.mapStringString(),
    raw: ch.string(),
  },
  { engine: "MergeTree()", orderBy: ["org_id", "user_id"] },
);

describe("coerceToTable", () => {
  const input = {
    org_id: "org_1",
    user_id: "u_42",
    source: "web",
    meta: { ip: "1.2.3.4", agent: { os: "mac" } },
  };

  it("routes known keys to their columns and unknown keys into attrs", () => {
    const { row } = coerceToTable(input, interactions);
    expect(row.org_id).toBe("org_1");
    expect(row.user_id).toBe("u_42");
    expect(row.attrs).toEqual({ source: "web", "meta.ip": "1.2.3.4", "meta.agent.os": "mac" });
  });

  it("reports the overflow keys", () => {
    const { overflowKeys } = coerceToTable(input, interactions);
    expect(overflowKeys.sort()).toEqual(["meta", "source"]);
  });

  it("sets raw to the JSON-stringified input", () => {
    const { row } = coerceToTable(input, interactions);
    expect(row.raw).toBe(JSON.stringify(input));
  });

  it("emits an empty attrs map when every key matches a column", () => {
    const { row, overflowKeys } = coerceToTable({ org_id: "o", user_id: "u" }, interactions);
    expect(row.attrs).toEqual({});
    expect(overflowKeys).toEqual([]);
  });

  it("treats input keys literally named attrs/raw as overflow, not column clobbers", () => {
    const { row, overflowKeys } = coerceToTable(
      { org_id: "o", user_id: "u", attrs: { x: 1 }, raw: "nope" },
      interactions,
    );
    expect(overflowKeys.sort()).toEqual(["attrs", "raw"]);
    expect(row.attrs).toEqual({ "attrs.x": "1", raw: "nope" });
    expect(row.raw).toBe(
      JSON.stringify({ org_id: "o", user_id: "u", attrs: { x: 1 }, raw: "nope" }),
    );
  });

  it("honors a custom catch-all column name", () => {
    const t = clickhouseTable(
      "t",
      { id: ch.string(), extra: ch.mapStringString() },
      { engine: "MergeTree()", orderBy: ["id"] },
    );
    const { row, overflowKeys } = coerceToTable({ id: "1", foo: "bar" }, t, { catchAll: "extra" });
    expect(row.extra).toEqual({ foo: "bar" });
    expect(overflowKeys).toEqual(["foo"]);
  });

  it("drops unmatched values when the table has no catch-all, but still reports them", () => {
    const t = clickhouseTable("t", { id: ch.string() }, { engine: "MergeTree()", orderBy: ["id"] });
    const { row, overflowKeys } = coerceToTable({ id: "1", foo: "bar" }, t);
    expect(row).toEqual({ id: "1" });
    expect(overflowKeys).toEqual(["foo"]);
  });
});
