// @smooai/clickhouse-kit — safe-by-construction primitives for user-defined,
// multi-tenant schemas. When column names + types come from untrusted input
// (a customer config, a DB row, JSON), these enforce the boundaries so the happy
// path makes SQL injection and unbounded tables IMPOSSIBLE, not merely discouraged:
//
//   - a bounded, allowlisted type system (reject anything not explicitly allowed),
//   - identifier validation + backtick-quoting (a name can't inject SQL),
//   - reserved-column handling, and size bounds (max columns, max identifier length).
//
// Higher-level runtime construction (clickhouseTableFromSpec / flexibleTable) builds
// on these — they own the boundary so every consumer is safe by default.

import { ChColumn } from "./kit";
import { z } from "zod";

/** Thrown when untrusted schema input violates a safety rule. */
export class SchemaSafetyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SchemaSafetyError";
  }
}

// ── Bounds ───────────────────────────────────────────────────────────────────
export interface SchemaLimits {
  /** Max columns a single table may declare. */
  readonly maxColumns: number;
  /** Max length of a table or column identifier. */
  readonly maxIdentifierLength: number;
}

export const DEFAULT_LIMITS: SchemaLimits = { maxColumns: 1024, maxIdentifierLength: 128 };

/** Throw unless `count` is within the column-count bound. */
export function assertColumnCount(count: number, limits: SchemaLimits = DEFAULT_LIMITS): void {
  if (count < 1) throw new SchemaSafetyError("a table must declare at least one column");
  if (count > limits.maxColumns)
    throw new SchemaSafetyError(`too many columns: ${count} > ${limits.maxColumns}`);
}

// ── Identifier safety ─────────────────────────────────────────────────────────
// A deliberately strict allowlist: ASCII letter/underscore start, then
// letters/digits/underscores. Safe names need no quoting at all; we still quote
// on render as defense-in-depth. Anything outside this (dots, spaces, quotes,
// backticks, unicode, leading digits, SQL metacharacters) is rejected.
const IDENTIFIER_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Validate a table/column identifier against the strict allowlist + length bound. Returns it on success. */
export function validateIdentifier(
  name: string,
  kind: "table" | "column" | "identifier" = "identifier",
  limits: SchemaLimits = DEFAULT_LIMITS,
): string {
  if (typeof name !== "string" || name.length === 0)
    throw new SchemaSafetyError(`empty ${kind} name`);
  if (name.length > limits.maxIdentifierLength)
    throw new SchemaSafetyError(
      `${kind} name too long: ${name.length} > ${limits.maxIdentifierLength}`,
    );
  if (!IDENTIFIER_RE.test(name))
    throw new SchemaSafetyError(
      `invalid ${kind} name ${JSON.stringify(name)}: must match ${IDENTIFIER_RE.source}`,
    );
  return name;
}

/** Backtick-quote an identifier for rendering, escaping embedded backticks (defense-in-depth). */
export function quoteIdentifier(name: string): string {
  return `\`${name.replace(/`/g, "``")}\``;
}

// ── Reserved columns ──────────────────────────────────────────────────────────
// The hybrid/flexible table shape reserves these for the catch-all + raw payload.
// User-supplied columns may not collide with them.
export const DEFAULT_RESERVED_COLUMNS: readonly string[] = ["attrs", "raw"];

/** Throw if `name` is reserved. */
export function assertNotReserved(
  name: string,
  reserved: readonly string[] = DEFAULT_RESERVED_COLUMNS,
): void {
  if (reserved.includes(name))
    throw new SchemaSafetyError(`column name ${JSON.stringify(name)} is reserved`);
}

// ── Type allowlist ────────────────────────────────────────────────────────────
// The ONLY column types a runtime/user spec may request. Everything else
// (Decimal, FixedString, Tuple, Enum, Nested, arbitrary expressions, …) is rejected.
export const ALLOWED_SCALAR_TYPES = [
  "String",
  "UUID",
  "Bool",
  "Date",
  "DateTime",
  "DateTime64",
  "Int8",
  "Int16",
  "Int32",
  "Int64",
  "UInt8",
  "UInt16",
  "UInt32",
  "UInt64",
  "Float32",
  "Float64",
  "JSON",
] as const;

export type AllowedScalarType = (typeof ALLOWED_SCALAR_TYPES)[number];

/**
 * A column type, as supplied by untrusted input — a JSON-friendly recursive shape.
 * Scalars are strings; the allowed wrappers/containers are single-key objects:
 *   'String' | 'Int32' | 'JSON'
 *   { nullable: <type> }            → Nullable(<type>)
 *   { lowCardinality: <type> }      → LowCardinality(<type>)
 *   { array: 'String' }             → Array(String)
 *   { map: ['String', 'String'] }   → Map(String, String)
 */
export type ColumnTypeSpec =
  | AllowedScalarType
  | { readonly nullable: ColumnTypeSpec }
  | { readonly lowCardinality: ColumnTypeSpec }
  | { readonly array: "String" }
  | { readonly map: readonly ["String", "String"] };

function scalarColumn(type: AllowedScalarType): ChColumn {
  switch (type) {
    case "String":
      return new ChColumn<string>("String", z.string());
    case "UUID":
      return new ChColumn<string>("UUID", z.string());
    case "Bool":
      return new ChColumn<boolean>("Bool", z.boolean());
    case "Date":
    case "DateTime":
      return new ChColumn<string>(type, z.string());
    case "DateTime64":
      return new ChColumn<string>("DateTime64(3)", z.string(), true);
    case "Int8":
    case "Int16":
    case "Int32":
    case "UInt8":
    case "UInt16":
    case "UInt32":
      return new ChColumn<number>(type, z.number().int());
    case "Int64":
    case "UInt64":
      return new ChColumn<string>(type, z.string()); // JS-safe: read 64-bit ints as strings
    case "Float32":
    case "Float64":
      return new ChColumn<number>(type, z.number());
    case "JSON":
      return new ChColumn<Record<string, unknown>>("JSON", z.record(z.string(), z.unknown()));
  }
}

/**
 * Build a `ChColumn` from an untrusted type spec, enforcing the allowlist. Throws
 * `SchemaSafetyError` for any disallowed type. This is the single gate that maps
 * outside input to a column — there is no path to an arbitrary type string.
 */
export function columnFromTypeSpec(spec: ColumnTypeSpec): ChColumn {
  if (typeof spec === "string") {
    if (!(ALLOWED_SCALAR_TYPES as readonly string[]).includes(spec)) {
      throw new SchemaSafetyError(
        `disallowed column type ${JSON.stringify(spec)} (allowed: ${ALLOWED_SCALAR_TYPES.join(", ")} + nullable/lowCardinality/array/map wrappers)`,
      );
    }
    return scalarColumn(spec);
  }
  if (spec === null || typeof spec !== "object") {
    throw new SchemaSafetyError(`invalid column type spec: ${JSON.stringify(spec)}`);
  }
  if ("nullable" in spec) {
    const inner = columnFromTypeSpec(spec.nullable);
    return new ChColumn(`Nullable(${inner.chType})`, inner.zodType.nullable(), inner.isDateTime64);
  }
  if ("lowCardinality" in spec) {
    const inner = columnFromTypeSpec(spec.lowCardinality);
    return new ChColumn(`LowCardinality(${inner.chType})`, inner.zodType, inner.isDateTime64);
  }
  if ("array" in spec) {
    if (spec.array !== "String")
      throw new SchemaSafetyError(
        `only Array(String) is allowed, got Array(${JSON.stringify(spec.array)})`,
      );
    return new ChColumn<string[]>("Array(String)", z.array(z.string()));
  }
  if ("map" in spec) {
    if (!Array.isArray(spec.map) || spec.map[0] !== "String" || spec.map[1] !== "String") {
      throw new SchemaSafetyError("only Map(String, String) is allowed");
    }
    return new ChColumn<Record<string, string>>(
      "Map(String, String)",
      z.record(z.string(), z.string()),
    );
  }
  throw new SchemaSafetyError(`unrecognized column type spec: ${JSON.stringify(spec)}`);
}
