import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { applyTheme, getInitialTheme, type Theme } from "./theme";
import { LoginPage } from "./components/LoginPage";
import { CockpitView } from "./components/CockpitView";
import { BriefingView } from "./components/BriefingView";
import { SettingsPanel } from "./components/SettingsPanel";
import { ActivityLogPanel } from "./components/ActivityLogPanel";
import { useSimSession } from "./hooks/useSimSession";
import type { ActiveFlightInfo, LoginResult } from "./types";

type SessionStatus =
  | { kind: "loading" }
  | { kind: "loggedOut" }
  | { kind: "loggedIn"; session: LoginResult };

type Tab = "cockpit" | "briefing" | "log" | "settings";

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
  const [tab, setTab] = useState<Tab>("briefing");
  const [debugMode, setDebugMode] = useState<boolean>(() => loadDebugMode());
  const { status: simStatus, snapshot: simSnapshot } = useSimSession();
  const simState = simStatus?.state ?? "disconnected";
  const [activeFlight, setActiveFlight] = useState<ActiveFlightInfo | null>(
    null,
  );

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

  // Centralised active-flight polling. Lives at the top so both the
  // Cockpit and the Briefing tab see the same state without duplicate
  // IPC calls. Cockpit auto-becomes the default tab once a flight
  // shows up; Briefing is the default while idle.
  useEffect(() => {
    if (status.kind !== "loggedIn") return;
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;
    async function poll() {
      try {
        const flight = await invoke<ActiveFlightInfo | null>("flight_status");
        if (cancelled) return;
        setActiveFlight(flight);
      } catch {
        // ignore
      }
    }
    void poll();
    timer = setInterval(poll, 2000);
    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [status.kind]);

  // Auto-switch to the cockpit tab the first time an active flight
  // appears (resume on startup, or just-started flight). The user can
  // still manually switch back to briefing afterwards — we only force
  // the switch on the rising edge.
  const [hadActiveFlight, setHadActiveFlight] = useState(false);
  useEffect(() => {
    if (activeFlight && !hadActiveFlight) {
      setTab("cockpit");
      setHadActiveFlight(true);
    }
    if (!activeFlight && hadActiveFlight) {
      setHadActiveFlight(false);
    }
  }, [activeFlight, hadActiveFlight]);

  async function handleLogout() {
    try {
      await invoke("phpvms_logout");
    } catch {
      // Drop in-memory session even if the keyring call fails.
    }
    setStatus({ kind: "loggedOut" });
    setTab("briefing");
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
        <div className="app__brand">
          <h1>{t("app.name")}</h1>
          <p className="tagline">{t("app.tagline")}</p>
        </div>
        <div className="app__status-pills">
          <span
            className={`status-pill status-pill--${
              phpvmsConnected ? "online" : "offline"
            }`}
            title={
              phpvmsConnected
                ? t("status.phpvms_connected")
                : t("status.phpvms_disconnected")
            }
          >
            <span className="status-pill__dot" />
            {t("status.phpvms")}
          </span>
          <span
            className={`status-pill status-pill--${
              simConnected ? "online" : simConnecting ? "connecting" : "offline"
            }`}
            title={
              simConnected
                ? t("status.simulator_connected")
                : simConnecting
                  ? t("status.simulator_connecting")
                  : t("status.simulator_disconnected")
            }
          >
            <span className="status-pill__dot" />
            {t("status.simulator")}
          </span>
        </div>
      </header>

      {showTabs && (
        <nav className="tabs" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "cockpit"}
            className={`tab ${tab === "cockpit" ? "tab--active" : ""}`}
            onClick={() => setTab("cockpit")}
          >
            {t("tabs.cockpit")}
            {activeFlight && <span className="tab__badge" aria-hidden="true" />}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "briefing"}
            className={`tab ${tab === "briefing" ? "tab--active" : ""}`}
            onClick={() => setTab("briefing")}
          >
            {t("tabs.briefing")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "log"}
            className={`tab ${tab === "log" ? "tab--active" : ""}`}
            onClick={() => setTab("log")}
          >
            {t("tabs.log")}
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

      {status.kind === "loggedIn" && tab === "cockpit" && (
        <CockpitView
          session={status.session}
          activeFlight={activeFlight}
          setActiveFlight={setActiveFlight}
          simSnapshot={simSnapshot}
          onSwitchToBriefing={() => setTab("briefing")}
        />
      )}

      {status.kind === "loggedIn" && tab === "briefing" && (
        <BriefingView
          session={status.session}
          activeFlight={activeFlight}
          setActiveFlight={setActiveFlight}
          onLogout={handleLogout}
          simState={simState}
          simSnapshot={simSnapshot}
        />
      )}

      {status.kind === "loggedIn" && tab === "log" && <ActivityLogPanel />}

      {status.kind === "loggedIn" && tab === "settings" && (
        <SettingsPanel
          debugMode={debugMode}
          onDebugModeChange={handleDebugModeChange}
          theme={theme}
          onThemeChange={setTheme}
          simStatus={simStatus}
        />
      )}
    </main>
  );
}

export default App;
