// TypeScript half of the cross-language parity corpus.
//
// Loads `spec/parity-corpus.json` — the SAME file `crates/clickhouse-kit/tests/parity.rs`
// loads — so the two ports of flatten / the type allowlist / identifier validation are
// checked against one committed set of expectations instead of two hand-mirrored ones.
//
// That distinction is not academic. Before this corpus existed the two flatteners
// disagreed about what `maxDepth` counts, and both suites were green: `flatten.test.ts`
// asserted TS's behaviour and `flatten.rs`'s tests asserted Rust's, and nothing compared
// them. A case added here must pass in every language or the build goes red.

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { flattenRecord } from "../flatten";
import { type ColumnTypeSpec, columnFromTypeSpec, validateIdentifier } from "../safety";

const corpusPath = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../spec/parity-corpus.json",
);

interface FlattenCase {
  name: string;
  input: Record<string, unknown>;
  options: { maxDepth: number; maxKeys: number; delimiter: string };
  expected: Record<string, string>;
}
interface ColumnTypeCase {
  name: string;
  spec: unknown;
  chType?: string;
  rejected?: boolean;
}
interface IdentifierCase {
  value: string;
  valid: boolean;
  why?: string;
}

const corpus = JSON.parse(readFileSync(corpusPath, "utf8")) as {
  flatten: { cases: FlattenCase[] };
  columnTypes: { cases: ColumnTypeCase[] };
  identifiers: { cases: IdentifierCase[] };
};

describe("parity corpus", () => {
  // A corpus nobody loads reads as a guarantee while proving nothing, and an
  // empty `cases` array would make every `it.each` below silently vacuous.
  it("is loaded and non-empty", () => {
    expect(corpus.flatten.cases.length).toBeGreaterThan(5);
    expect(corpus.columnTypes.cases.length).toBeGreaterThan(15);
    expect(corpus.identifiers.cases.length).toBeGreaterThan(10);
  });

  describe("flattenRecord", () => {
    it.each(corpus.flatten.cases)("$name", ({ input, options, expected }) => {
      expect(flattenRecord(input, options)).toEqual(expected);
    });
  });

  describe("columnFromTypeSpec", () => {
    it.each(corpus.columnTypes.cases)("$name", ({ spec, chType, rejected }) => {
      if (rejected) {
        // Rust rejects these by having no enum variant to deserialize into;
        // TS rejects them by throwing. Different mechanism, same guarantee.
        expect(() => columnFromTypeSpec(spec as ColumnTypeSpec)).toThrow();
        return;
      }
      expect(columnFromTypeSpec(spec as ColumnTypeSpec).chType).toBe(chType);
    });
  });

  describe("validateIdentifier", () => {
    it.each(corpus.identifiers.cases)("$value ($why)", ({ value, valid }) => {
      if (valid) {
        expect(validateIdentifier(value, "column")).toBe(value);
      } else {
        expect(() => validateIdentifier(value, "column")).toThrow();
      }
    });
  });
});
