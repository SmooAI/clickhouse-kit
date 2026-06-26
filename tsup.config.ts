import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  outDir: "dist",
  clean: true,
  dts: true,
  format: ["cjs", "esm"],
  sourcemap: true,
  target: "es2022",
  treeShaking: true,
});
