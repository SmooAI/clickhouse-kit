import { describe, expect, it } from "vitest";
import { checkClickHouseDrift, expectedColumns, type LiveColumn } from "../check";
import type { ClickHouseClient } from "../migrate";
import { events } from "./fixtures";

// A mock client serving canned system.columns rows keyed by the table param.
function mockClient(byTable: Record<string, LiveColumn[]>): ClickHouseClient {
  return {
    command: async () => undefined,
    query: async (args) => {
      const table = (args.query_params?.t as string) ?? "";
      return { json: async () => byTable[table] ?? [] };
    },
  };
}

const matchingLive: LiveColumn[] = [...expectedColumns(events).entries()].map(([name, type]) => ({
  name,
  type,
}));

describe("checkClickHouseDrift", () => {
  it("reports NO drift when the live schema matches the kit", async () => {
    const result = await checkClickHouseDrift(mockClient({ events: matchingLive }), [events]);
    expect(result.checked).toEqual(["events"]);
    expect(result.drift).toEqual([]);
  });

  it("tolerates cosmetic type spacing", async () => {
    const squished = matchingLive.map((c) => ({ ...c, type: c.type.replace(/, /g, ",") }));
    const result = await checkClickHouseDrift(mockClient({ events: squished }), [events]);
    expect(result.drift).toEqual([]);
  });

  it("flags a missing table", async () => {
    const result = await checkClickHouseDrift(mockClient({}), [events]);
    expect(result.drift).toEqual([{ table: "events", kind: "missing_table" }]);
  });

  it("flags a missing column", async () => {
    const dropped = matchingLive.filter((c) => c.name !== "count");
    const result = await checkClickHouseDrift(mockClient({ events: dropped }), [events]);
    expect(result.drift).toContainEqual({
      table: "events",
      kind: "missing_column",
      column: "count",
      expected: "UInt16",
    });
  });

  it("flags an extra column", async () => {
    const extra = [...matchingLive, { name: "rogue", type: "String" }];
    const result = await checkClickHouseDrift(mockClient({ events: extra }), [events]);
    expect(result.drift).toContainEqual({
      table: "events",
      kind: "extra_column",
      column: "rogue",
      actual: "String",
    });
  });

  it("flags a type mismatch", async () => {
    const retyped = matchingLive.map((c) => (c.name === "value" ? { ...c, type: "Float32" } : c));
    const result = await checkClickHouseDrift(mockClient({ events: retyped }), [events]);
    expect(result.drift).toContainEqual({
      table: "events",
      kind: "type_mismatch",
      column: "value",
      expected: "Float64",
      actual: "Float32",
    });
  });
});
