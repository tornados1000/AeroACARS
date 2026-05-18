import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./i18n";
import "./App.css";
import { applyTheme, getInitialTheme } from "./theme";
import { SkinProvider } from "./components/SkinContext";
import { initSentry, getConsent, Sentry } from "./lib/sentry";
import { invoke } from "@tauri-apps/api/core";

// v0.9.0 (#GlitchTip): Sentry-Init MUSS frueh laufen, sonst gehen
// Bootstrap-Fehler im weissen Bildschirm unter. Init macht KEIN
// Network-Call wenn DSN nicht in Build-Time-Env gesetzt war.
initSentry(typeof __APP_VERSION__ !== "undefined" ? __APP_VERSION__ : "0.0.0");

// Initial-Consent-Sync zum Rust-Backend (Default = false). Damit folgt
// der Backend-Atomic dem gespeicherten Pilot-Choice ohne dass der User
// erst zu Settings muss. Wenn der Command nicht existiert (z.B. alter
// Backend-Build), ignorieren wir den Fehler — Default bleibt aus.
void invoke("error_reporting_set_consent", { enabled: getConsent() }).catch(
  () => undefined,
);

applyTheme(getInitialTheme());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Sentry.ErrorBoundary
      fallback={({ resetError }) => (
        <div style={{ padding: 24, fontFamily: "system-ui, sans-serif" }}>
          <h2 style={{ marginTop: 0 }}>Etwas ist schiefgegangen</h2>
          <p>
            AeroACARS hat einen unerwarteten Fehler abgefangen. Wenn die
            anonyme Fehler-Telemetrie aktiv ist, wurde der Fehler bereits
            gemeldet — anderenfalls passiert nichts.
          </p>
          <button onClick={resetError}>Erneut versuchen</button>
        </div>
      )}
    >
      <SkinProvider>
        <App />
      </SkinProvider>
    </Sentry.ErrorBoundary>
  </React.StrictMode>,
);
