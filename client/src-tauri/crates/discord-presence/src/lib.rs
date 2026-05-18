//! v0.9.0 (#Discord-RPC) — Discord Rich Presence Manager fuer AeroACARS.
//!
//! Spec: docs/spec/v0.9.0-discord-rich-presence.md
//!     + docs/spec/v0.9.0-telemetry-contract.md Sektion 9 (DSGVO-Gates)
//!
//! Ziel:
//!   - Pilot-Flugstatus live im Discord-Profil sichtbar
//!     ("GSG3184 · EDDB → KMRH" / "CRUISE · A320 · FL360")
//!   - 60s Heartbeat + Sofort-Update bei Phase-Wechsel
//!   - Default AUS (Opt-In), Privacy-by-Default (DSGVO Art. 6 (1) a)
//!   - Graceful Fallback wenn Discord nicht installiert / nicht offen
//!   - Optionaler Anonym-Modus ("GSG-Flight" statt "GSG3184")
//!
//! Architektur:
//!   - `DiscordPresenceManager` ist die einzige oeffentliche Surface
//!     fuer den Tauri-Code; alles andere ist intern oder pure-fn (in format.rs).
//!   - State-Machine im Mutex-geschuetzten Inner, async-Methoden auf der
//!     Outer-Wrapper-Struct sodass Tauri-Commands sie await-en koennen.
//!   - Heartbeat-Loop laeuft als spawned tokio-task; wird auf disable/shutdown
//!     abgebrochen.

pub mod format;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Utc;
use discord_rich_presence::{
    activity::{Activity, Assets, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Discord-App-ID. Wird zur Build-Zeit aus env `AEROACARS_DISCORD_APP_ID`
/// gelesen — wenn nicht gesetzt, faellt der Manager auf einen Sentinel
/// zurueck der den Init absichtlich scheitern laesst (= No-Op-Pfad).
///
/// Der VA-Owner registriert die App einmal auf https://discord.com/developers/applications
/// und setzt die ID als env in der Release-CI (GitHub-Actions-Secret).
pub fn client_id() -> &'static str {
    option_env!("AEROACARS_DISCORD_APP_ID").unwrap_or("")
}

/// Heartbeat-Intervall — Discord erwartet einen Refresh innerhalb ~120s,
/// sonst loescht es die Activity. 60s gibt Sicherheits-Marge ohne nervig zu sein.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Test-Presence bleibt 15s sichtbar, dann automatisches Clear (Spec §UI/Test-Button).
const TEST_PRESENCE_DURATION: Duration = Duration::from_secs(15);

// ─── Public Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordPresenceSettings {
    /// Master-Toggle: AN -> Pipe oeffnen + Presence senden. Default AUS.
    pub enabled: bool,
    /// Wenn AN: "GSG3184" -> "GSG-Flight" (Route bleibt sichtbar).
    pub anonymize_callsign: bool,
    /// Wenn AN: Profil-Button-Anker auf der Presence (= phpVMS-URL).
    /// Default AN — Pilot kann es ausschalten wenn ihm das Profil-Link unangenehm ist.
    #[serde(default = "default_true")]
    pub show_profile_button: bool,
}

impl Default for DiscordPresenceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            anonymize_callsign: false,
            show_profile_button: true,
        }
    }
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize)]
pub struct DiscordPresenceState {
    pub status: PresenceStatus,
    pub last_connect_attempt_at: Option<String>,
    pub last_update_at: Option<String>,
    pub client_id: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Connected,
    NotFound,
    Disabled,
    Error,
}

#[derive(Debug, Clone)]
pub struct PresenceInput {
    pub callsign: String,
    pub dep_icao: String,
    pub arr_icao: String,
    pub aircraft: String,
    pub altitude_ft: Option<i32>,
    pub phase: FlightPhase,
    pub sim: SimKind,
    /// Flight-Start als UNIX-Timestamp. Discord nutzt das fuer die „X min"-Anzeige.
    pub start_unix: i64,
    /// Optionaler phpVMS-Profil-Link fuer den "Open Profile"-Button.
    pub profile_url: Option<String>,
}

/// 18 kanonische Phasen aus Telemetry-Contract Sektion 1.3.
/// Pflicht: jede dieser Phasen MUSS in format::phase_to_label() ein Mapping haben.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightPhase {
    Preflight,
    Boarding,
    Pushback,
    TaxiOut,
    TakeoffRoll,
    Takeoff,
    RejectedTakeoff,
    Climb,
    Cruise,
    Descent,
    Approach,
    Final,
    Landing,
    GoAround,
    TaxiIn,
    Arrived,
    BlocksOn,
    Deboarding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimKind {
    Msfs2020,
    Msfs2024,
    Xplane11,
    Xplane12,
    Prepar3d,
    Unknown,
}

// ─── Manager ───────────────────────────────────────────────────────────────

pub struct DiscordPresenceManager {
    inner: Mutex<Inner>,
}

struct Inner {
    client: Option<DiscordIpcClient>,
    settings: DiscordPresenceSettings,
    state: DiscordPresenceState,
    last_input: Option<PresenceInput>,
    sim_lost: bool,
    heartbeat_handle: Option<JoinHandle<()>>,
}

impl DiscordPresenceManager {
    pub fn new(settings: DiscordPresenceSettings) -> Arc<Self> {
        let id = client_id();
        let initial_status = if settings.enabled {
            PresenceStatus::Disabled // wird beim enable() auf Connected/NotFound aktualisiert
        } else {
            PresenceStatus::Disabled
        };
        Arc::new(Self {
            inner: Mutex::new(Inner {
                client: None,
                settings,
                state: DiscordPresenceState {
                    status: initial_status,
                    last_connect_attempt_at: None,
                    last_update_at: None,
                    client_id: id.to_string(),
                    error_message: None,
                },
                last_input: None,
                sim_lost: false,
                heartbeat_handle: None,
            }),
        })
    }

    /// Snapshot fuer das Frontend (Settings-Status-Anzeige).
    pub async fn current_state(&self) -> DiscordPresenceState {
        let inner = self.inner.lock().await;
        inner.state.clone()
    }

    pub async fn current_settings(&self) -> DiscordPresenceSettings {
        let inner = self.inner.lock().await;
        inner.settings.clone()
    }

    /// Pilot hat den Master-Toggle umgelegt oder ein anderes Setting geaendert.
    /// Bei `enabled=true` und !connected → connect + send + start heartbeat.
    /// Bei `enabled=false` → clear activity + close pipe + stop heartbeat.
    /// Bei Anonym-Toggle change wird das naechste set_activity neu berechnet
    /// (kein Sofort-Push noetig, der Heartbeat-Tick reicht).
    pub async fn apply_settings(self: &Arc<Self>, new_settings: DiscordPresenceSettings) -> Result<()> {
        let was_enabled;
        {
            let mut inner = self.inner.lock().await;
            was_enabled = inner.settings.enabled;
            inner.settings = new_settings.clone();
        }

        match (was_enabled, new_settings.enabled) {
            (false, true) => self.enable().await,
            (true, false) => self.disable().await,
            (true, true) => {
                // Anonym-Toggle oder Profile-Button-Toggle geaendert → naechster Heartbeat reicht.
                // Aber: sofortiger Re-Push damit der Pilot die Aenderung gleich sieht.
                self.push_current_activity().await.ok();
                Ok(())
            }
            (false, false) => Ok(()),
        }
    }

    async fn enable(self: &Arc<Self>) -> Result<()> {
        let id = client_id();
        if id.is_empty() {
            let mut inner = self.inner.lock().await;
            inner.state.status = PresenceStatus::Error;
            inner.state.error_message = Some(
                "Discord-App-ID ist nicht gesetzt (Build ohne AEROACARS_DISCORD_APP_ID). \
                 Bitte aus einem offiziellen Release-Build laufen."
                    .to_string(),
            );
            return Err(anyhow!("missing Discord client_id"));
        }

        let now = Utc::now().to_rfc3339();
        let connect_result = {
            let mut inner = self.inner.lock().await;
            inner.state.last_connect_attempt_at = Some(now);

            // Discord-IPC-Client erstellen + connecten.
            // In discord-rich-presence v0.2.5 returns DiscordIpcClient::new ein
            // Result (kann beim Pipe-Path-Build scheitern); .connect() ist sync.
            match DiscordIpcClient::new(id).and_then(|mut client| {
                client.connect().map(|_| client)
            }) {
                Ok(client) => {
                    inner.client = Some(client);
                    inner.state.status = PresenceStatus::Connected;
                    inner.state.error_message = None;
                    info!(client_id=%id, "[discord-rpc] connected");
                    Ok(())
                }
                Err(e) => {
                    inner.state.status = PresenceStatus::NotFound;
                    inner.state.error_message = Some(format!("{e:?}"));
                    debug!(error=?e, "[discord-rpc] connect failed (Discord not running?)");
                    Err(anyhow!("connect failed: {e:?}"))
                }
            }
        };

        // Heartbeat-Loop unabhaengig vom initialen Connect-Erfolg starten —
        // wenn Discord spaeter aufgeht und der naechste Tick connectet,
        // klappt die Pipe und die Presence kommt nach. (Implementierung
        // im Heartbeat-Loop: wenn !client.is_some(), retry-connect.)
        let weak = Arc::downgrade(self);
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
            tick.tick().await; // erster Tick ist instant, ueberspringen
            loop {
                tick.tick().await;
                let Some(mgr) = weak.upgrade() else { return };
                mgr.heartbeat_step().await;
            }
        });

        {
            let mut inner = self.inner.lock().await;
            if let Some(old) = inner.heartbeat_handle.replace(handle) {
                old.abort();
            }
        }

        // initiale Presence pushen wenn schon ein Flug aktiv ist
        self.push_current_activity().await.ok();

        // Connect-Result transparent durchreichen damit der Caller log/UI weiss
        connect_result
    }

    async fn disable(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if let Some(handle) = inner.heartbeat_handle.take() {
            handle.abort();
        }
        if let Some(mut client) = inner.client.take() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
        inner.state.status = PresenceStatus::Disabled;
        inner.state.error_message = None;
        info!("[discord-rpc] disabled");
        Ok(())
    }

    /// Sauberer App-Quit-Cleanup. Idempotent.
    pub async fn shutdown(&self) -> Result<()> {
        self.disable().await
    }

    /// Pilot hat einen Flug gestartet → initialer Presence-Push.
    pub async fn set_flight(self: &Arc<Self>, input: PresenceInput) -> Result<()> {
        {
            let mut inner = self.inner.lock().await;
            inner.last_input = Some(input);
        }
        self.push_current_activity().await
    }

    /// Nur Phase + Altitude haben sich geaendert (alter PresenceInput recyceln).
    pub async fn update_phase(
        self: &Arc<Self>,
        phase: FlightPhase,
        altitude_ft: Option<i32>,
    ) -> Result<()> {
        {
            let mut inner = self.inner.lock().await;
            let Some(last) = inner.last_input.as_mut() else {
                return Ok(()); // kein aktiver Flug
            };
            if last.phase == phase && last.altitude_ft == altitude_ft {
                return Ok(()); // nichts geaendert
            }
            last.phase = phase;
            last.altitude_ft = altitude_ft;
        }
        self.push_current_activity().await
    }

    /// MQTT/Sim Disconnect → "⚠ Sim getrennt"-Suffix in der State-Zeile (LE8).
    pub async fn set_sim_lost(self: &Arc<Self>, lost: bool) -> Result<()> {
        {
            let mut inner = self.inner.lock().await;
            if inner.sim_lost == lost {
                return Ok(());
            }
            inner.sim_lost = lost;
        }
        self.push_current_activity().await
    }

    /// Flight ist beendet → Presence komplett weg.
    pub async fn clear_flight(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.last_input = None;
        inner.sim_lost = false;
        if let Some(client) = inner.client.as_mut() {
            client.clear_activity().map_err(|e| anyhow!("{e:?}"))?;
            inner.state.last_update_at = Some(Utc::now().to_rfc3339());
        }
        Ok(())
    }

    /// Test-Presence: 15s sichtbar mit Dummy-Daten, dann auto-clear.
    /// Pilot kann verifizieren dass Discord die App sieht ohne dafuer einen
    /// echten Flug starten zu muessen.
    pub async fn send_test_presence(self: &Arc<Self>) -> Result<()> {
        let test = PresenceInput {
            callsign: "AERO-TEST".to_string(),
            dep_icao: "EDDB".to_string(),
            arr_icao: "EDDK".to_string(),
            aircraft: "A320".to_string(),
            altitude_ft: Some(36000),
            phase: FlightPhase::Cruise,
            sim: SimKind::Msfs2024,
            start_unix: Utc::now().timestamp(),
            profile_url: None,
        };
        self.set_flight(test).await?;

        // Auto-Clear-Task
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(TEST_PRESENCE_DURATION).await;
            if let Some(mgr) = weak.upgrade() {
                let _ = mgr.clear_flight().await;
            }
        });
        Ok(())
    }

    /// Heartbeat-Step: re-send Presence (Discord-Timeout-Schutz). Wenn der Client
    /// disconnected ist, versuchen wir einen Re-Connect — Discord koennte
    /// inzwischen aufgegangen sein.
    async fn heartbeat_step(self: &Arc<Self>) {
        // Lock kurz halten — nur lesen ob enabled
        {
            let inner = self.inner.lock().await;
            if !inner.settings.enabled {
                return;
            }
        }

        // Re-connect-Versuch wenn keine Verbindung
        {
            let mut inner = self.inner.lock().await;
            if inner.client.is_none() {
                if let Ok(client) = DiscordIpcClient::new(client_id())
                    .and_then(|mut c| c.connect().map(|_| c))
                {
                    inner.client = Some(client);
                    inner.state.status = PresenceStatus::Connected;
                    inner.state.error_message = None;
                    info!("[discord-rpc] reconnected via heartbeat");
                }
            }
        }
        let _ = self.push_current_activity().await;
    }

    /// Baut das Activity-Objekt aus inner.last_input und sendet es.
    /// No-Op wenn kein aktiver Flug oder kein verbundener Client.
    async fn push_current_activity(self: &Arc<Self>) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let Some(input) = inner.last_input.clone() else {
            return Ok(()); // nichts zu zeigen
        };
        let settings = inner.settings.clone();
        let sim_lost = inner.sim_lost;

        let Some(client) = inner.client.as_mut() else {
            return Ok(()); // Discord offline; Heartbeat retry's spaeter
        };

        let details = format::build_details(&input, settings.anonymize_callsign);
        let state = format::build_state(&input, sim_lost);
        let small_image = format::sim_to_asset_key(input.sim).unwrap_or("");
        let small_tooltip = format::sim_to_tooltip(input.sim);

        let mut assets = Assets::new().large_image(format::ASSET_LOGO);
        if !small_image.is_empty() {
            assets = assets.small_image(small_image).small_text(small_tooltip);
        }

        let activity = Activity::new()
            .details(&details)
            .state(&state)
            .assets(assets)
            .timestamps(Timestamps::new().start(input.start_unix));

        match client.set_activity(activity) {
            Ok(_) => {
                inner.state.status = PresenceStatus::Connected;
                inner.state.error_message = None;
                inner.state.last_update_at = Some(Utc::now().to_rfc3339());
                Ok(())
            }
            Err(e) => {
                // Pipe abgerissen → Client wegwerfen, naechster Heartbeat versucht's
                inner.client = None;
                inner.state.status = PresenceStatus::NotFound;
                inner.state.error_message = Some(format!("{e:?}"));
                warn!(error=?e, "[discord-rpc] set_activity failed, will retry");
                Err(anyhow!("{e:?}"))
            }
        }
    }
}
