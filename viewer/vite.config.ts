import { defineConfig } from "vite";

export default defineConfig({
  base: "./",
  // The sim worker dynamically imports the wasm-bindgen glue, which needs ES
  // module workers (the default "iife" format cannot code-split).
  worker: { format: "es" },
  build: { target: "es2022" },
});
