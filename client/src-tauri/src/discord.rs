//! Discord-Integration (Webhook + zukünftig Rich Presence).
//!
//! v0.4.0 — hardcoded für GSG. Pilot-Opt-Out via Settings, sonst keine
//! UI-Konfig (kein phpVMS-Modul). Webhook-URL + RP-App-ID stehen als
//! Konstanten in diesem Modul; wer den Repo forked tauscht sie hier
//! aus.
//!
//! ## Sicherheit
//!
//! Die Webhook-URL ist quasi ein Passwort — wer das URL-Token hat
//! kann in den Channel posten. Das ist akzeptabel solange das Repo
//! privat bleibt. **NIE auf einem öffentlichen Repo committen** — bei
//! Public-Open-Source sofort Webhook in Discord rotieren und die
//! URL über ein anderes Channel verteilen (env var, downloaded
//! config from server, …).

use std::sync::Mutex;

use ::discord_rich_presence::{
    activity::{Activity, Assets, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use serde::Serialize;

/// GSG-Discord-Webhook (für Auto-Posts: Takeoff / Landing / PIREP /
/// Divert). Bei Fork für eine andere VA: hier austauschen.
const WEBHOOK_URL: &str = "https://discord.com/api/webhooks/1501257121235468418/0sLGmj9-LY4sPfsL0iw7s0TQRmI9qgyYcTek147kR0igU1__IXjx8hXafAl-fPmdOp7Q";

/// Discord-App-ID für Rich Presence. Registriert auf
/// discord.com/developers als „AeroACARS / German Sky Group". Wird
/// im Discord-Client als App-Name angezeigt neben dem RP-Status.
/// Bei Fork: eigene App registrieren und ID hier austauschen.
#[allow(dead_code)] // Wiring in den Streamer kommt in v0.4.5
const RICH_PRESENCE_APP_ID: &str = "1340818636616634489";

/// Welcher Lifecycle-Event Discord posten soll. Mapped 1:1 auf einen
/// Embed mit eigener Farbe + Icon, gebaut von [`build_embed`].
#[derive(Debug, Clone, Copy)]
pub enum EventKind {
    Takeoff,
    Landing,
    PirepFiled,
    Divert,
}

/// Felder die zum Bauen aller Embeds reichen. Wir füllen nur was wir
/// haben — Felder die für ein Event keinen Sinn machen (z.B. Score
/// bei Takeoff) bleiben `None`.
#[derive(Debug, Clone, Default)]
pub struct EventContext {
    pub callsign: String,            // "RYR100" — Flight-Number-Teil
    pub airline_icao: String,        // "RYR" — für Logo-Lookup-Fallback
    pub airline_logo_url: Option<String>, // direkt von phpVMS bid.flight.airline.logo
    pub dpt_icao: String,            // "LOWS"
    pub arr_icao: String,            // "EDDB"  (oder Divert-Ziel)
    pub planned_arr_icao: Option<String>, // bei Divert: ursprüngliches Ziel
    pub aircraft_type: Option<String>,    // "B738"
    pub aircraft_reg: Option<String>,     // "EI-ENI"
    pub pilot_ident: Option<String>,      // "GSG0001"
    pub pilot_name: Option<String>,       // "Thomas K"
    pub block_fuel_kg: Option<f32>,       // bei Takeoff
    pub planned_block_fuel_kg: Option<f32>,
    pub tow_kg: Option<f32>,              // bei Takeoff
    pub landing_rate_fpm: Option<f32>,    // bei Landing
    pub score: Option<i32>,               // bei Landing/PIREP
    pub distance_nm: Option<f64>,
    pub fuel_used_kg: Option<f32>,
    pub flight_time_min: Option<i32>,
}

/// Discord Embed (Webhook-API-kompatibel). Wir nutzen mehr Felder als
/// die Minimalversion — `author` für die phasenspezifische Headline,
/// `thumbnail` für ein Icon rechts oben, `fields` für eine saubere
/// Key/Value-Tabelle (Discord rendert die als Grid). Discord ignoriert
/// jedes Feld das wir nicht setzen — `Option<...>` + `skip_serializing_if`
/// hält den Wire-Body sauber.
#[derive(Debug, Serialize)]
struct Embed {
    title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    color: u32,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail: Option<EmbedImage>,
    /// Großes Bild unten im Embed — wir nutzen das für die
    /// **Airline-Livery / das Airline-Logo** so wie's die VAs üblich
    /// machen (Aircalin-Hibiscus, FedEx-Schriftzug etc.). Lookup
    /// per Airline-ICAO in [`airline_logo_url`].
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<EmbedImage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fields: Vec<EmbedField>,
    footer: EmbedFooter,
}

#[derive(Debug, Serialize)]
struct EmbedAuthor {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct EmbedImage {
    url: String,
}

#[derive(Debug, Serialize)]
struct EmbedField {
    name: String,
    value: String,
    /// `true` = Feld nimmt nur ⅓ der Zeile (max 3 inline pro Zeile).
    /// Wir nutzen Inline für kurze Werte (ICAO, Score, Fuel) und nicht-
    /// inline für längere Texte (Pilot, Notes).
    inline: bool,
}

#[derive(Debug, Serialize)]
struct EmbedFooter {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct WebhookPayload {
    embeds: Vec<Embed>,
}

/// Per-Event-Bilder. Bei Fork: hier eigene URLs eintragen — z.B. ins
/// eigene Repo unter `.discord-assets/` legen und per
/// `raw.githubusercontent.com/<owner>/<repo>/main/.discord-assets/<...>.png`
/// referenzieren. URLs müssen über HTTPS erreichbar sein, Discord
/// cached die für ~24h.
const TAKEOFF_THUMBNAIL: &str =
    "https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/.discord-assets/takeoff.png";
const LANDING_THUMBNAIL: &str =
    "https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/.discord-assets/landing.png";
const PIREP_THUMBNAIL: &str =
    "https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/.discord-assets/pirep.png";
const DIVERT_THUMBNAIL: &str =
    "https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/.discord-assets/divert.png";
/// VA-Logo (oben rechts als Thumbnail). GSG-Crest passt zum Stil
/// anderer VA-Bots (siehe Pilot-Screenshots vom 2026-05-05).
const VA_THUMBNAIL: &str =
    "https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/.discord-assets/gsg-logo.png";

/// AeroACARS-Logo für die `author.icon_url`-Zeile oben links im Embed.
/// Klein, neben „GSG0001 - Pilot Name".
const AUTHOR_ICON: &str =
    "https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/.discord-assets/aeroacars-logo.png";

// Airline-Logos kommen ab v0.4.0 ausschließlich aus phpVMS — die VA
// pflegt sie im Admin-Panel unter Airlines, phpVMS serviert die
// resultierende URL via `bid.flight.airline.logo`. Kein Hardcoding,
// kein externes CDN nötig. Wenn die VA für eine Airline kein Logo
// hochgeladen hat, fehlt das große Bild im Discord-Embed — Author,
// Title und Fields bleiben aber sauber lesbar.

/// Event → Discord-Embed. Layout angelehnt an die VA-Bot-Konvention
/// (Pilot-Vorlage Screenshots 2026-05-05): Author-Zeile oben mit
/// **Pilot-ID + Name**, Title in Bold als „Flight X has landed",
/// strukturierte Felder in 3-Spalten (Dep / Arr / Equipment), unten
/// das **Airline-Logo als großes Bild**, Color-Stripe je Phase.
fn build_embed(kind: EventKind, ctx: &EventContext) -> Embed {
    let now = chrono::Utc::now().to_rfc3339();
    let footer_text = format!("AeroACARS v{}", env!("CARGO_PKG_VERSION"));

    // Author-Bar: phpVMS-Pilot-ID + Name (Format „GSG0001 - Thomas K"),
    // klein oben links. Wenn keine Pilot-Daten da sind, Fallback auf
    // generischen AeroACARS-Header.
    let author_name = match (&ctx.pilot_ident, &ctx.pilot_name) {
        (Some(id), Some(name)) => format!("{} - {}", id, name),
        (Some(id), None) => id.clone(),
        (None, Some(name)) => name.clone(),
        (None, None) => "AeroACARS Pilot".to_string(),
    };
    let author = EmbedAuthor {
        name: author_name,
        icon_url: Some(AUTHOR_ICON.to_string()),
    };

    // Thumbnail (rechts oben) = VA-Logo. So kennt jeder im Channel
    // die Marke wieder. Bei Wunsch nach phasen-spezifischen Action-
    // Photos: hier einen Switch auf TAKEOFF_THUMBNAIL etc. setzen.
    let thumbnail = Some(EmbedImage {
        url: VA_THUMBNAIL.to_string(),
    });

    // Großes Bild unten = Airline-Logo aus phpVMS. URL kommt direkt
    // aus der Bid-Relation (`bid.flight.airline.logo`, serviert von
    // der VA-Webseite). Kein Fallback — wenn die VA kein Logo
    // hochgeladen hat, bleibt das Embed ohne großes Bild.
    let _ = (TAKEOFF_THUMBNAIL, LANDING_THUMBNAIL, PIREP_THUMBNAIL, DIVERT_THUMBNAIL);
    let image = ctx
        .airline_logo_url
        .clone()
        .filter(|s| !s.is_empty())
        .map(|url| EmbedImage { url });

    let (title, color, description, fields) = match kind {
        EventKind::Takeoff => build_takeoff(ctx),
        EventKind::Landing => build_landing(ctx),
        EventKind::PirepFiled => build_pirep_filed(ctx),
        EventKind::Divert => build_divert(ctx),
    };

    Embed {
        title,
        description,
        color,
        timestamp: now,
        author: Some(author),
        thumbnail,
        image,
        fields,
        footer: EmbedFooter {
            text: footer_text,
            icon_url: None,
        },
    }
}

fn fmt_kg(kg: f32) -> String {
    let n = kg.round() as i64;
    // Tausender-Trennzeichen (deutsche Schreibweise).
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.insert(0, '.');
        }
        out.insert(0, c);
    }
    out + " kg"
}

/// Equipment-Spalte: `B738 (EI-ENI)` oder nur `B738` falls keine Reg.
fn fmt_equipment(ctx: &EventContext) -> String {
    match (&ctx.aircraft_type, &ctx.aircraft_reg) {
        (Some(t), Some(r)) => format!("{} ({})", t, r),
        (Some(t), None) => t.clone(),
        (None, Some(r)) => r.clone(),
        (None, None) => "—".to_string(),
    }
}

/// 3-Spalten-Block: Dep.Airport / Arr.Airport / Equipment. Plus
/// 2-Spalten: Flight Time / Distance. Layout matched die VA-Bot-
/// Konvention der Pilot-Screenshots.
fn push_route_fields(fields: &mut Vec<EmbedField>, ctx: &EventContext) {
    fields.push(EmbedField {
        name: "Dep.Airport".to_string(),
        value: ctx.dpt_icao.clone(),
        inline: true,
    });
    fields.push(EmbedField {
        name: "Arr.Airport".to_string(),
        value: ctx.arr_icao.clone(),
        inline: true,
    });
    fields.push(EmbedField {
        name: "Equipment".to_string(),
        value: fmt_equipment(ctx),
        inline: true,
    });
}

fn push_time_distance_fields(
    fields: &mut Vec<EmbedField>,
    flight_time_min: Option<i32>,
    distance_nm: Option<f64>,
) {
    if let Some(t) = flight_time_min {
        let h = t / 60;
        let m = t % 60;
        fields.push(EmbedField {
            name: "Flight Time".to_string(),
            value: format!("{}h {:02}m", h, m),
            inline: true,
        });
    }
    if let Some(d) = distance_nm {
        fields.push(EmbedField {
            name: "Distance".to_string(),
            value: format!("{:.2} nmi", d),
            inline: true,
        });
    }
}

fn build_takeoff(ctx: &EventContext) -> (String, u32, String, Vec<EmbedField>) {
    let title = format!("✈️ Flight {}/C.PF took off", ctx.callsign);
    let description = String::new();
    let mut fields = Vec::new();
    push_route_fields(&mut fields, ctx);

    // Block / TOW: zwei zusätzliche Inline-Felder unter der Standard-
    // Route-Zeile. Helps Pilot/Discord-Audience sehen ob das Loadsheet
    // gepasst hat (Δ-Tag wenn Plan vorhanden).
    if let Some(actual) = ctx.block_fuel_kg {
        let val = if let Some(plan) = ctx.planned_block_fuel_kg {
            let delta = actual - plan;
            let sign = if delta >= 0.0 { "+" } else { "" };
            format!("{}\nPlan {} ({}{:.0})", fmt_kg(actual), fmt_kg(plan), sign, delta)
        } else {
            fmt_kg(actual)
        };
        fields.push(EmbedField {
            name: "Block-Fuel".to_string(),
            value: val,
            inline: true,
        });
    }
    if let Some(tow) = ctx.tow_kg {
        fields.push(EmbedField {
            name: "TOW".to_string(),
            value: fmt_kg(tow),
            inline: true,
        });
    }
    (
        title,
        0x22c55e, // green-500
        description,
        fields,
    )
}

fn build_landing(ctx: &EventContext) -> (String, u32, String, Vec<EmbedField>) {
    let title = format!("🛬 Flight {}/C.PF has landed", ctx.callsign);
    let description = String::new();
    let mut fields = Vec::new();
    push_route_fields(&mut fields, ctx);
    push_time_distance_fields(&mut fields, ctx.flight_time_min, ctx.distance_nm);

    if let Some(rate) = ctx.landing_rate_fpm {
        let symbol = match rate.abs() as i32 {
            0..=149 => "⭐ Butter",
            150..=349 => "✓ Sauber",
            350..=599 => "⚠️ Hart",
            _ => "🚨 Sehr hart",
        };
        fields.push(EmbedField {
            name: "Landing-Rate".to_string(),
            value: format!("{:.0} fpm  {}", rate, symbol),
            inline: true,
        });
    }
    if let Some(s) = ctx.score {
        let stars = if s >= 90 {
            "⭐⭐⭐"
        } else if s >= 70 {
            "⭐⭐"
        } else if s >= 50 {
            "⭐"
        } else {
            "—"
        };
        fields.push(EmbedField {
            name: "Score".to_string(),
            value: format!("**{}**/100  {}", s, stars),
            inline: true,
        });
    }
    (
        title,
        0xf97316, // orange-500 — passt visuell zu den Screenshots
        description,
        fields,
    )
}

fn build_pirep_filed(ctx: &EventContext) -> (String, u32, String, Vec<EmbedField>) {
    let title = format!("📋 Flight {}/C.PF Filed", ctx.callsign);
    let description = String::new();
    let mut fields = Vec::new();
    push_route_fields(&mut fields, ctx);
    push_time_distance_fields(&mut fields, ctx.flight_time_min, ctx.distance_nm);

    if let Some(s) = ctx.score {
        fields.push(EmbedField {
            name: "Score".to_string(),
            value: format!("**{}**/100", s),
            inline: true,
        });
    }
    (
        title,
        0xa855f7, // purple-500 — neutral für „filed"
        description,
        fields,
    )
}

fn build_divert(ctx: &EventContext) -> (String, u32, String, Vec<EmbedField>) {
    let planned_str = ctx
        .planned_arr_icao
        .as_deref()
        .unwrap_or(&ctx.arr_icao);
    let title = format!("⚠️ Flight {}/C.PF Divert", ctx.callsign);
    let description = format!(
        "**{}** ist nach **{}** divertiert (geplant war **{}**).",
        ctx.callsign, ctx.arr_icao, planned_str
    );
    let mut fields = Vec::new();
    fields.push(EmbedField {
        name: "Dep.Airport".to_string(),
        value: ctx.dpt_icao.clone(),
        inline: true,
    });
    fields.push(EmbedField {
        name: "Geplant".to_string(),
        value: planned_str.to_string(),
        inline: true,
    });
    fields.push(EmbedField {
        name: "Tatsächlich".to_string(),
        value: ctx.arr_icao.clone(),
        inline: true,
    });
    fields.push(EmbedField {
        name: "Equipment".to_string(),
        value: fmt_equipment(ctx),
        inline: true,
    });
    fields.push(EmbedField {
        name: "Status".to_string(),
        value: "🔍 PENDING\n(VA-Admin-Review)".to_string(),
        inline: true,
    });
    push_time_distance_fields(&mut fields, ctx.flight_time_min, ctx.distance_nm);
    (
        title,
        0xf59e0b, // amber-500
        description,
        fields,
    )
}

/// Schickt den Event-Embed via HTTP-POST an die GSG-Webhook-URL.
/// Best-effort — Fehler werden als WARN geloggt aber niemals propagiert
/// nach oben (Discord soll *nie* den Flug-Workflow blocken). Wenn der
/// Pilot Discord-Posts deaktiviert hat (Settings) ruft niemand diese
/// Funktion auf — der Filter passiert beim Caller.
pub async fn post_event(kind: EventKind, ctx: EventContext) {
    let payload = WebhookPayload {
        embeds: vec![build_embed(kind, &ctx)],
    };
    // Eigener kleiner reqwest-Client damit wir nicht den phpVMS-API-
    // Client recyclen müssen (würde unnötig den Auth-Header tragen).
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Discord webhook: HTTP-Client-Bau fehlgeschlagen");
            return;
        }
    };
    match client.post(WEBHOOK_URL).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(?kind, "Discord webhook posted");
        }
        Ok(resp) => {
            tracing::warn!(
                status = %resp.status(),
                ?kind,
                "Discord webhook: non-2xx response"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, ?kind, "Discord webhook: post failed");
        }
    }
}

// ===========================================================================
// Rich Presence
// ===========================================================================

/// Was der Pilot in Discord neben dem AeroACARS-App-Eintrag sieht.
/// Mapped 1:1 auf eine Discord-Activity. Enum statt `Option<...>`-
/// Tuple weil das Mapping-Verhalten je nach Phase unterschiedlich
/// ist (z.B. Cruise zeigt FL, Boarding zeigt's nicht).
#[derive(Debug, Clone)]
#[allow(dead_code)] // Wiring in den Streamer kommt in v0.4.5
pub enum PresenceState {
    /// Kein aktiver Flug — Pilot scrollt durch Bids o.ä.
    Idle,
    /// Aktiver Flug. Phase-Label kommt rein als bereits-übersetzter
    /// Kurzstring (z.B. "Boarding", "Cruising").
    InFlight {
        callsign: String,        // "RYR100"
        dpt_icao: String,        // "LOWS"
        arr_icao: String,        // "EDDB"
        phase_label: String,     // "Cruising" / "Boarding" / etc.
        aircraft_type: Option<String>, // "B738"
        cruise_fl: Option<i32>,  // FL340 → 340
        started_at_unix: i64,    // für "elapsed XX:YY"-Anzeige in Discord
    },
}

/// Singleton-ähnlicher Service: hält die IPC-Connection zum lokalen
/// Discord-Client. Kapselt Reconnect-Logik (Discord kann während des
/// Flugs gestartet/beendet werden) und thread-safe Mutex-Wrapping.
///
/// Der Service ist optional — fehlt Discord oder schlägt der Connect
/// fehl, loggen wir warn und gehen weiter. Niemals fatal.
///
/// **WIP-Status (v0.4.0+):** Foundation gebaut, Wiring in den Streamer
/// kommt in v0.4.5. `#[allow(dead_code)]` damit der vorbereitete
/// Code keine Warnings im Build wirft bis er dann live geht.
#[allow(dead_code)]
pub struct RichPresenceService {
    /// `None` wenn Discord nicht erreichbar ist (nicht installiert,
    /// nicht laufend, IPC-Pipe noch nicht offen). `Some` sobald die
    /// erste Verbindung steht — bleibt drin auch wenn Discord
    /// zwischenzeitlich beendet wird, der nächste `set_*`-Call
    /// versucht's neu.
    client: Mutex<Option<DiscordIpcClient>>,
}

#[allow(dead_code)] // Wiring in den Streamer kommt in v0.4.5
impl RichPresenceService {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
        }
    }

    /// Versucht Connect falls noch keine Verbindung steht. Idempotent.
    /// Returnt true wenn die Verbindung danach (vermutlich) lebt.
    fn ensure_connected(&self) -> bool {
        let mut guard = self.client.lock().expect("rp client lock");
        if guard.is_some() {
            return true;
        }
        let mut c = DiscordIpcClient::new(RICH_PRESENCE_APP_ID);
        match c.connect() {
            Ok(()) => {
                tracing::info!(app_id = RICH_PRESENCE_APP_ID, "Discord RP connected");
                *guard = Some(c);
                true
            }
            Err(e) => {
                tracing::debug!(error = %e, "Discord RP connect failed (Discord nicht offen?)");
                false
            }
        }
    }

    /// Hauptmethode: setzt den Discord-Status auf `state`. Bei IPC-
    /// Fehler wird die Connection verworfen und der nächste Call
    /// versucht reconnect.
    pub fn set(&self, state: PresenceState) {
        if !self.ensure_connected() {
            return;
        }
        let activity = build_activity(&state);
        let mut guard = self.client.lock().expect("rp client lock");
        let Some(client) = guard.as_mut() else { return };
        if let Err(e) = client.set_activity(activity) {
            tracing::warn!(error = %e, "Discord RP set_activity failed — reconnecting next time");
            // Drop the broken client; ensure_connected will rebuild.
            *guard = None;
        }
    }

    /// Discord-Status komplett entfernen (App schließt).
    pub fn clear(&self) {
        let mut guard = self.client.lock().expect("rp client lock");
        if let Some(client) = guard.as_mut() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
        *guard = None;
    }
}

impl Default for RichPresenceService {
    fn default() -> Self {
        Self::new()
    }
}

/// Map `PresenceState` → Discord-Activity. Discord rendert:
///
/// ```text
/// AeroACARS  (App-Name aus dem Developer-Portal)
///   <details>     ← top line, bigger
///   <state>       ← bottom line, smaller
///   xx:yy elapsed (wenn timestamp gesetzt)
/// ```
///
/// Wir nutzen `details` für die wichtigste Info (Callsign + Route)
/// und `state` für Phase + zusätzliche Details (FL, Aircraft).
#[allow(dead_code)] // Wiring kommt in v0.4.5
fn build_activity(state: &PresenceState) -> Activity<'_> {
    match state {
        PresenceState::Idle => Activity::new()
            .details("Browsing bids")
            .state("Idle on the apron"),
        PresenceState::InFlight {
            callsign,
            dpt_icao,
            arr_icao,
            phase_label,
            aircraft_type,
            cruise_fl,
            started_at_unix,
        } => {
            let details = format!("{} · {} → {}", callsign, dpt_icao, arr_icao);
            let mut state_line = phase_label.clone();
            if let Some(fl) = cruise_fl {
                state_line.push_str(&format!(" · FL{:03}", fl));
            }
            if let Some(ac) = aircraft_type {
                state_line.push_str(&format!(" · {}", ac));
            }
            let mut activity = Activity::new()
                .details(string_leak(details))
                .state(string_leak(state_line))
                .timestamps(Timestamps::new().start(*started_at_unix))
                .assets(
                    Assets::new()
                        // Asset key — falls auf discord.com/developers
                        // unter "Rich Presence Art Assets" hochgeladen
                        // (sonst zeigt Discord nichts oder den App-
                        // Avatar-Fallback). Keys können wir später
                        // ergänzen — Activity-Builder ignoriert nicht-
                        // existente keys silently.
                        .large_image("aeroacars_logo")
                        .large_text("AeroACARS"),
                );
            // Kein .small_image / .small_text — wir haben aktuell nur
            // ein Asset. Zukünftig: Sim-Logo (MSFS / X-Plane) als
            // small_image für mehr Kontext.
            let _ = &mut activity; // silence unused-mut if branch shrinks
            activity
        }
    }
}

/// Activity API in `discord-rich-presence` v1 nimmt `&'a str` —
/// damit wir Strings dynamisch bauen können und sie überleben den
/// `set_activity`-Call brauchen wir Heap-Pointer mit static lifetime.
/// `Box::leak` ist hier akzeptabel: wir leaken pro RP-Update ~50
/// Bytes, im Schnitt 1 Update pro Phase-Change → über einen ganzen
/// Flug ~10 Updates × 100 Bytes = 1 KB total. Wer 1000 Flüge in einer
/// Session macht ohne den Client neu zu starten, leakt 1 MB — okay.
#[allow(dead_code)] // Wiring kommt in v0.4.5
fn string_leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
