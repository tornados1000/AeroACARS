import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { applyTheme, getInitialTheme, type Theme } from "./theme";
import { LoginPage } from "./components/LoginPage";
import { CockpitView } from "./components/CockpitView";
import { BriefingView } from "./components/BriefingView";
import { SettingsPanel } from "./components/SettingsPanel";
import { ActivityLogPanel } from "./components/ActivityLogPanel";
import { AboutPanel } from "./components/AboutPanel";
import { LandingPanel } from "./components/LandingPanel";
import { UpdateButton } from "./components/UpdateButton";
import { LiveRecordingIndicator } from "./components/LiveRecordingIndicator";
import { useSimSession } from "./hooks/useSimSession";
import type { ActiveFlightInfo, LoginResult, Profile } from "./types";

type SessionStatus =
  | { kind: "loading" }
  | { kind: "loggedOut" }
  | { kind: "loggedIn"; session: LoginResult };

type Tab = "cockpit" | "briefing" | "landing" | "log" | "settings" | "about";

const DEBUG_STORAGE_KEY = "aeroacars.debug";
const AUTO_FILE_STORAGE_KEY = "aeroacars.autoFile";
const AUTO_START_STORAGE_KEY = "aeroacars.autoStart";
const AUTO_DELETE_LOGS_STORAGE_KEY = "aeroacars.autoDeleteFlightLogs";
/** Days threshold for the auto-purge sweep. Mirrors the wording of the
 *  Settings hint — keep both in sync if you ever change it. */
const AUTO_DELETE_LOGS_DAYS = 30;

function loadDebugMode(): boolean {
  return localStorage.getItem(DEBUG_STORAGE_KEY) === "1";
}

function saveDebugMode(value: boolean) {
  localStorage.setItem(DEBUG_STORAGE_KEY, value ? "1" : "0");
}

/** Auto-file the PIREP when the FSM reaches Arrived. Default ON —
 *  removes one click from the happy path. Disabling forces the
 *  pilot to hit "Flug beenden" manually, useful when they want to
 *  inspect mass / fuel / activity log before submitting. */
function loadAutoFile(): boolean {
  const v = localStorage.getItem(AUTO_FILE_STORAGE_KEY);
  // Default true: only persisted "0" disables.
  return v !== "0";
}

function saveAutoFile(value: boolean) {
  localStorage.setItem(AUTO_FILE_STORAGE_KEY, value ? "1" : "0");
}

/** Auto-start a flight when the aircraft is parked at the departure
 *  airport of one of the user's bids. Default OFF — opt-in feature.
 *  Backend watcher polls every 3 s while enabled. */
function loadAutoStart(): boolean {
  return localStorage.getItem(AUTO_START_STORAGE_KEY) === "1";
}

function saveAutoStart(value: boolean) {
  localStorage.setItem(AUTO_START_STORAGE_KEY, value ? "1" : "0");
}

/** Sweep stale per-flight JSONL recorder files at app start. Default
 *  ON — keeps the app data dir from accumulating gigabytes over years
 *  of flying. Pilots who want every flight retained forever can flip
 *  the toggle off in Settings → Speicher. Only persisted "0" disables. */
function loadAutoDeleteFlightLogs(): boolean {
  return localStorage.getItem(AUTO_DELETE_LOGS_STORAGE_KEY) !== "0";
}

function saveAutoDeleteFlightLogs(value: boolean) {
  localStorage.setItem(AUTO_DELETE_LOGS_STORAGE_KEY, value ? "1" : "0");
}

/**
 * Map a SimKind string to the brand label shown on the top-right
 * status pill. Pilots want to see WHICH sim is connected, not the
 * generic word "Simulator". Falls back to "SIM" when nothing is
 * selected so the pill never goes blank.
 */
function simKindLabel(kind: string | undefined): string {
  switch (kind) {
    case "msfs2024":
    case "msfs2020":
      return "MSFS";
    case "xplane11":
    case "xplane12":
      return "X-PLANE";
    case "off":
      return "SIM OFF";
    default:
      return "SIM";
  }
}

function App() {
  const { t } = useTranslation();
  const [theme, setTheme] = useState<Theme>(() => getInitialTheme());
  const [status, setStatus] = useState<SessionStatus>({ kind: "loading" });
  const [tab, setTab] = useState<Tab>("briefing");
  const [debugMode, setDebugMode] = useState<boolean>(() => loadDebugMode());
  const [autoFile, setAutoFile] = useState<boolean>(() => loadAutoFile());
  const [autoStart, setAutoStart] = useState<boolean>(() => loadAutoStart());
  const [autoDeleteFlightLogs, setAutoDeleteFlightLogs] = useState<boolean>(
    () => loadAutoDeleteFlightLogs(),
  );
  const { status: simStatus, snapshot: simSnapshot } = useSimSession();

  // Auto-purge stale flight log files once per app launch when the
  // toggle is on. Fires on mount only — re-toggling at runtime doesn't
  // re-sweep (next launch will). 30-day threshold matches the Settings
  // hint copy.
  useEffect(() => {
    if (!loadAutoDeleteFlightLogs()) return;
    void invoke("flight_logs_purge_older_than", {
      olderThanDays: AUTO_DELETE_LOGS_DAYS,
    }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sync the persisted auto-start flag to the Rust backend on every
  // mount/change. Backend default is OFF; localStorage is the source
  // of truth. Without this sync, the watcher wouldn't run after a
  // restart even though the toggle is enabled in the UI.
  useEffect(() => {
    void invoke("auto_start_set_enabled", { enabled: autoStart }).catch(() => {});
  }, [autoStart]);
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
      // Flight just ended (filed / cancelled / discarded). PhpVMS
      // updates the pilot's `curr_airport` server-side as part of an
      // accepted PIREP, but our cached LoginResult never sees it
      // unless we re-fetch. Without this, the dashboard "Aktueller
      // Airport" stays at the old value until the next app restart.
      void invoke<Profile | null>("phpvms_refresh_profile")
        .then((fresh) => {
          if (!fresh) return;
          setStatus((prev) =>
            prev.kind === "loggedIn"
              ? {
                  kind: "loggedIn",
                  session: { ...prev.session, profile: fresh },
                }
              : prev,
          );
        })
        .catch(() => {});
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

  function handleAutoFileChange(next: boolean) {
    setAutoFile(next);
    saveAutoFile(next);
  }

  function handleAutoStartChange(next: boolean) {
    setAutoStart(next);
    saveAutoStart(next);
    // The useEffect on autoStart will sync the toggle to the Rust
    // watcher. setState alone won't fire it (React batches), so we
    // also pre-emptively call here for snappier UX.
    void invoke("auto_start_set_enabled", { enabled: next }).catch(() => {});
  }

  function handleAutoDeleteFlightLogsChange(next: boolean) {
    setAutoDeleteFlightLogs(next);
    saveAutoDeleteFlightLogs(next);
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
          <UpdateButton />
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
            {simKindLabel(simStatus?.kind)}
          </span>
          {activeFlight && (
            <LiveRecordingIndicator
              lastPositionAt={activeFlight.last_position_at}
              queuedCount={activeFlight.queued_position_count}
              positionCount={activeFlight.position_count}
            />
          )}
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
            aria-selected={tab === "landing"}
            className={`tab ${tab === "landing" ? "tab--active" : ""}`}
            onClick={() => setTab("landing")}
          >
            {t("tabs.landing")}
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
          <button
            type="button"
            role="tab"
            aria-selected={tab === "about"}
            className={`tab ${tab === "about" ? "tab--active" : ""}`}
            onClick={() => setTab("about")}
          >
            {t("tabs.about")}
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
          autoFile={autoFile}
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

      {status.kind === "loggedIn" && tab === "landing" && <LandingPanel />}

      {status.kind === "loggedIn" && tab === "log" && <ActivityLogPanel />}

      {status.kind === "loggedIn" && tab === "settings" && (
        <SettingsPanel
          debugMode={debugMode}
          onDebugModeChange={handleDebugModeChange}
          autoFile={autoFile}
          onAutoFileChange={handleAutoFileChange}
          autoStart={autoStart}
          onAutoStartChange={handleAutoStartChange}
          autoDeleteFlightLogs={autoDeleteFlightLogs}
          onAutoDeleteFlightLogsChange={handleAutoDeleteFlightLogsChange}
          theme={theme}
          onThemeChange={setTheme}
          simStatus={simStatus}
          activeFlight={activeFlight}
        />
      )}

      {status.kind === "loggedIn" && tab === "about" && <AboutPanel />}
    </main>
  );
}

export default App;
