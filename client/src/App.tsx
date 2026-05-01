import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { applyTheme, getInitialTheme, type Theme } from "./theme";
import { LoginPage } from "./components/LoginPage";
import { Dashboard } from "./components/Dashboard";
import { SettingsPanel } from "./components/SettingsPanel";
import type { LoginResult, SimConnectionState } from "./types";

type SessionStatus =
  | { kind: "loading" }
  | { kind: "loggedOut" }
  | { kind: "loggedIn"; session: LoginResult };

type Tab = "dashboard" | "settings";

const DEBUG_STORAGE_KEY = "cloudeacars.debug";

function loadDebugMode(): boolean {
  return localStorage.getItem(DEBUG_STORAGE_KEY) === "1";
}

function saveDebugMode(value: boolean) {
  localStorage.setItem(DEBUG_STORAGE_KEY, value ? "1" : "0");
}

function App() {
  const { t } = useTranslation();
  const [theme, setTheme] = useState<Theme>(() => getInitialTheme());
  const [status, setStatus] = useState<SessionStatus>({ kind: "loading" });
  const [tab, setTab] = useState<Tab>("dashboard");
  const [debugMode, setDebugMode] = useState<boolean>(() => loadDebugMode());
  const [simState, setSimState] = useState<SimConnectionState>("disconnected");

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const result = await invoke<LoginResult | null>("phpvms_load_session");
        if (cancelled) return;
        setStatus(
          result ? { kind: "loggedIn", session: result } : { kind: "loggedOut" },
        );
      } catch {
        if (!cancelled) setStatus({ kind: "loggedOut" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleLogout() {
    try {
      await invoke("phpvms_logout");
    } catch {
      // Drop in-memory session even if the keyring call fails.
    }
    setStatus({ kind: "loggedOut" });
    setTab("dashboard");
  }

  function handleDebugModeChange(next: boolean) {
    setDebugMode(next);
    saveDebugMode(next);
  }

  const phpvmsConnected = status.kind === "loggedIn";
  const simConnected = simState === "connected";
  const simConnecting = simState === "connecting";
  const showTabs = status.kind === "loggedIn";

  return (
    <main className="app">
      <header className="app__header">
        <div>
          <h1>{t("app.name")}</h1>
          <p className="tagline">{t("app.tagline")}</p>
        </div>
      </header>

      <section className="status-grid">
        <div
          className={`status-card status-card--${
            phpvmsConnected ? "online" : "offline"
          }`}
        >
          <span className="status-card__label">{t("status.phpvms")}</span>
          <span className="status-card__value">
            {phpvmsConnected
              ? t("status.phpvms_connected")
              : t("status.phpvms_disconnected")}
          </span>
        </div>
        <div
          className={`status-card status-card--${
            simConnected ? "online" : simConnecting ? "connecting" : "offline"
          }`}
        >
          <span className="status-card__label">{t("status.simulator")}</span>
          <span className="status-card__value">
            {simConnected
              ? t("status.simulator_connected")
              : simConnecting
                ? t("status.simulator_connecting")
                : t("status.simulator_disconnected")}
          </span>
        </div>
      </section>

      {showTabs && (
        <nav className="tabs" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "dashboard"}
            className={`tab ${tab === "dashboard" ? "tab--active" : ""}`}
            onClick={() => setTab("dashboard")}
          >
            {t("tabs.dashboard")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "settings"}
            className={`tab ${tab === "settings" ? "tab--active" : ""}`}
            onClick={() => setTab("settings")}
          >
            {t("tabs.settings")}
          </button>
        </nav>
      )}

      {status.kind === "loading" && (
        <section className="phase">
          <p>{t("status.checking_session")}</p>
        </section>
      )}

      {status.kind === "loggedOut" && (
        <LoginPage
          onSuccess={(s) => setStatus({ kind: "loggedIn", session: s })}
        />
      )}

      {status.kind === "loggedIn" && tab === "dashboard" && (
        <Dashboard
          session={status.session}
          onLogout={handleLogout}
          onSimStateChange={setSimState}
          debugMode={debugMode}
        />
      )}

      {status.kind === "loggedIn" && tab === "settings" && (
        <SettingsPanel
          debugMode={debugMode}
          onDebugModeChange={handleDebugModeChange}
          theme={theme}
          onThemeChange={setTheme}
        />
      )}
    </main>
  );
}

export default App;
