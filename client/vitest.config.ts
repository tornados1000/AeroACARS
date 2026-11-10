// v0.7.8 Phase 1: Vitest-Konfiguration fuer Component-Tests
// Spec: docs/spec/v0.7.8-landing-rate-explainability.md §8.0
//
// jsdom-Environment damit @testing-library/react funktioniert
// (DOM-APIs in node verfuegbar). Globals damit `describe`/`it`/`expect`
// ohne expliziten Import nutzbar sind — wie in den vorhandenen
// LandingPanel-Pattern.

import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    // exclude die Tauri-Side-Tests (die laufen via `cargo test`)
    exclude: ["**/node_modules/**", "**/dist/**", "**/src-tauri/**"],
  },
});
