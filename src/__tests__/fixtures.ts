// A generic example schema used across the tests — showcases the feature surface
// (LowCardinality, DateTime64-with-TTL-auto-wrap, Map, UUID, DEFAULT, skip index,
// a materialized view) without any consumer-specific coupling.

import { ch, clickhouseMaterializedView, clickhouseTable, createSelectSchema } from "../kit";

export const events = clickhouseTable(
  "events",
  {
    ts: ch.dateTime64(3),
    org_id: ch.lowCardinality(ch.string()),
    event_id: ch.uuid(),
    name: ch.string(),
    kind: ch.lowCardinality(ch.string()),
    value: ch.float64(),
    attributes: ch.mapStringString(),
    count: ch.uint16(),
    ingested_at: ch.dateTime().default("now()"),
  },
  {
    engine: "MergeTree()",
    partitionBy: "(org_id, toDate(ts))",
    orderBy: ["org_id", "ts", "event_id"],
    ttl: {
      column: "ts",
      moveToVolumeAfter: { interval: "14 DAY", volume: "cold" },
      deleteAfter: "90 DAY",
    },
    indexes: [{ name: "idx_name", expr: "name", type: "bloom_filter(0.01)", granularity: 1 }],
    settings: { storage_policy: "hot_cold", index_granularity: 8192 },
  },
);

export const eventsByDayMv = clickhouseMaterializedView("events_by_day_mv", {
  to: "events_by_day",
  asSelect: "SELECT org_id, toDate(ts) AS day, count() AS c FROM events GROUP BY org_id, day",
});

export const selectEventSchema = createSelectSchema(events);

export const validEventRow = {
  ts: "2026-06-25 12:00:00.000",
  org_id: "org_123",
  event_id: "00000000-0000-0000-0000-000000000000",
  name: "signup",
  kind: "server",
  value: 1.5,
  attributes: { source: "web" },
  count: 3,
  ingested_at: "2026-06-25 12:00:02",
};
