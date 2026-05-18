//! v0.9.0 (#GlitchTip) — Sentry-Init fuer den Tauri-Client.
//!
//! Spec: docs/spec/v0.9.0-glitchtip-self-hosted.md
//!     + docs/spec/v0.9.0-telemetry-contract.md Sektion 9 (Datenschutz-Gates)
//!
//! Init-Strategie:
//!   - DSN aus build-time env `AEROACARS_SENTRY_DSN` (siehe build.rs).
//!     Wenn leer → kein Init, alle Calls degradieren zu No-Ops.
//!   - Opt-In: Pilot muss explizit zustimmen (Settings-UI + First-Run-
//!     Banner). Bis dann: kein Event geht raus. Default = aus.
//!   - release tag: `aeroacars-client@${CARGO_PKG_VERSION}`
//!
//! Anonymisierung (Allowlist + Redaction):
//!   - `before_send`-Hook strippt alle nicht-allowlisted Tags
//!   - User-PII (email, username, ip_address) → None
//!   - Request-Daten (headers, body, query) komplett weg
//!   - Stack-Frame-Vars weg
//!
//! Privacy-Toggle:
//!   - `set_consent(true|false)` setzt globale Atomic, vor jedem
//!     before_send-Call gepruef. So kann der Pilot live ein/ausschalten,
//!     ohne dass wir Sentry restarten muessen — auch wenn Sentry
//!     re-init wird der DSN beim Re-init reagiert.

use sentry::protocol::{Event, Value};
use sentry::ClientInitGuard;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Pilot-Consent-Gate. Default `false`. Wird von der UI via Tauri-Command
/// `set_error_reporting_consent` gesetzt. Bei `false` wird jedes Event
/// im before_send verworfen.
static CONSENT: AtomicBool = AtomicBool::new(false);

/// Halt den Init-Guard so lange am Leben wie der Prozess. Drop = flush.
static SENTRY_GUARD: OnceLock<Option<ClientInitGuard>> = OnceLock::new();

/// Tag-Allowlist (Telemetry-Contract Sektion 1, Client-Subset).
fn allowed_tag_keys() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("app.component");
        s.insert("app.version");
        s.insert("os");
        s.insert("os.version");
        s.insert("simulator");
        s.insert("aircraft");
        s.insert("airport");
        s.insert("runway");
        s.insert("pirep.id");
        s.insert("callsign");
        // v0.9.0 QS-Hotfix: "route" entfernt — Privacy-Hint sagt "Route NICHT
        // gesendet", also auch nicht erlauben. War ohnehin von keinem Code-Pfad
        // gesetzt (Spec-future placeholder).
        s.insert("phase");
        s.insert("pilot.hash");
        s.insert("forensics.version");
        s.insert("error.code");
        s.insert("error.status_code");
        s.insert("error.kind");
        s.insert("feature.flag");
        s.insert("distance.bucket");
        s
    })
}

/// Baut die ClientOptions — wird sowohl im initial-init() als auch beim
/// Re-Opt-In (rebind_client) genutzt damit Sentry-Re-Init im selben App-Lauf
/// moeglich ist (siehe set_consent). Returns None wenn keine DSN konfiguriert.
fn build_options() -> Option<sentry::ClientOptions> {
    let dsn = option_env!("AEROACARS_SENTRY_DSN").unwrap_or("").trim();
    if dsn.is_empty() {
        return None;
    }
    let release = format!("aeroacars-client@{}", env!("CARGO_PKG_VERSION"));
    let environment = if cfg!(debug_assertions) {
        "development"
    } else {
        "production"
    };
    Some(sentry::ClientOptions {
        dsn: dsn.parse().ok(),
        release: Some(release.into()),
        environment: Some(environment.into()),
        send_default_pii: false,
        traces_sample_rate: 0.0,
        attach_stacktrace: true,
        before_send: Some(std::sync::Arc::new(|event| {
            // Consent-Gate
            if !CONSENT.load(Ordering::Relaxed) {
                return None;
            }
            Some(redact_event(event))
        })),
        ..Default::default()
    })
}

/// Initialisiert Sentry. No-Op wenn DSN nicht in env zur Build-Zeit gesetzt war.
///
/// MUSS frueh in `lib.rs::run()` aufgerufen werden, vor dem Tauri-Builder,
/// damit auch Bootstrap-Panics gefangen werden.
pub fn init() {
    SENTRY_GUARD.get_or_init(|| {
        let Some(options) = build_options() else {
            tracing::info!("[sentry] no DSN configured (AEROACARS_SENTRY_DSN at build-time); skipping init");
            return None;
        };
        let release_tag = options.release.as_ref().map(|c| c.to_string()).unwrap_or_default();
        let guard = sentry::init(options);

        // Initial-Scope: Komponenten-Tags
        sentry::configure_scope(|scope| {
            scope.set_tag("app.component", "client");
            scope.set_tag("app.version", env!("CARGO_PKG_VERSION"));
            scope.set_tag("os", std::env::consts::OS);
        });

        tracing::info!("[sentry] initialized for release {}", release_tag);
        Some(guard)
    });
}

/// Setzt den Pilot-Consent. Symmetrisch beide Richtungen:
///  - `false` → CONSENT-Atomic = false + Transport hart gedroppt
///    (`bind_client(None)`). Pending events im Transport-Buffer der
///    sentry-Lib gehen verloren statt rausgesendet zu werden. DS7
///    hart erfuellt: "ab Klick geht nichts mehr raus".
///  - `true`  → CONSENT-Atomic = true + (falls Client nicht mehr
///    gebunden ist) neuen Client mit identischen Options binden.
///    So funktioniert Re-Opt-In im selben App-Run, ohne Neustart.
///
/// QS-Hotfix Verlauf v0.9.x:
///  - v0.9.0 initial: set_consent(false) rief flush() — das pushed pending
///    Events AKTIV raus, exakt das Gegenteil von Opt-out.
///  - v0.9.0 Runde 1 (F3): flush() entfernt. Atomic-Gate verhindert
///    kuenftige Events, ABER pending im Transport-Buffer war noch da und
///    haette beim naechsten Tick rausgehen koennen (= P1-Rest-Risiko).
///  - v0.9.1 Runde 2 (F9): zusaetzlich Hub::current().bind_client(None)
///    → Transport wird gedroppt. Aber Re-Opt-In war damit kaputt
///    (= Settings-Hint "wirkt sofort, kein Neustart noetig" wurde fuer
///    den Re-Opt-In-Fall zur Luege).
///  - v0.9.1 Runde 3 (F12): symmetrischer Re-Init bei opt-in via
///    build_options() + Hub::bind_client(Arc::new(Client::from(options))).
///    Damit ist Hint wieder ehrlich: aus-an-aus-an funktioniert in einem
///    App-Lauf.
pub fn set_consent(enabled: bool) {
    CONSENT.store(enabled, Ordering::Relaxed);
    tracing::info!("[sentry] consent set to {}", enabled);
    if enabled {
        // Re-Init falls Client nicht mehr gebunden ist (= nach vorherigem
        // Opt-Out im selben App-Run). Wenn schon ein Client gebunden ist
        // (= initialer Init lief gerade, oder noch nie opt-out geklickt),
        // brauchen wir nichts zu tun — das Atomic-Gate reicht.
        let hub = sentry::Hub::current();
        if hub.client().is_none() {
            if let Some(options) = build_options() {
                let client = sentry::Client::from(options);
                hub.bind_client(Some(std::sync::Arc::new(client)));
                // Initial-Scope nochmal setzen — der frische Client hat keinen
                // Scope mehr von der ersten init()-Runde.
                sentry::configure_scope(|scope| {
                    scope.set_tag("app.component", "client");
                    scope.set_tag("app.version", env!("CARGO_PKG_VERSION"));
                    scope.set_tag("os", std::env::consts::OS);
                });
                tracing::info!("[sentry] re-bound after opt-in (within-app re-init)");
            }
        }
    } else {
        // bind_client(None) entfernt den Client aus dem Hub → der Drop des
        // Clients schliesst den Transport → pending Buffer wird verworfen
        // statt gesendet. Defense-in-Depth zusaetzlich zum before_send-Gate.
        sentry::Hub::current().bind_client(None);
    }
}

/// Anonymisiert ein Event: Tags-Allowlist + User-PII + Request-Daten + Frame-Vars.
pub fn redact_event(mut event: Event<'static>) -> Event<'static> {
    // 1. Tag-Allowlist
    let allowed = allowed_tag_keys();
    event.tags.retain(|k, _| allowed.contains(k.as_str()));

    // 2. User-PII (id nur wenn Hash-Format)
    if let Some(user) = event.user.as_mut() {
        let id_ok = user
            .id
            .as_ref()
            .map(|id| id.chars().all(|c| c.is_ascii_hexdigit()) && id.len() <= 16)
            .unwrap_or(false);
        if !id_ok {
            user.id = None;
        }
        user.email = None;
        user.username = None;
        user.ip_address = None;
        // Sonstige beliebige Felder weg
        user.other.clear();
    }

    // 3. Request-Daten: alles weg ausser method + url-ohne-query
    if let Some(req) = event.request.as_mut() {
        req.cookies = None;
        req.data = None;
        req.headers.clear();
        req.env.clear();
        req.query_string = None;
        if let Some(url) = req.url.as_ref() {
            // query-string raus
            let url_str = url.as_str();
            if let Some(idx) = url_str.find('?') {
                if let Ok(clean) = url_str[..idx].parse() {
                    req.url = Some(clean);
                }
            }
        }
    }

    // 4. Breadcrumbs: sensible data-keys redacten
    for crumb in event.breadcrumbs.iter_mut() {
        for (key, value) in crumb.data.iter_mut() {
            let lower = key.to_lowercase();
            if lower.contains("token")
                || lower.contains("password")
                || lower.contains("authorization")
                || lower.contains("cookie")
                || lower == "body"
                || lower == "request_body"
            {
                *value = Value::String("[REDACTED]".into());
            }
        }
    }

    // 5. Stack-Frame-Vars weg
    for ex in event.exception.iter_mut() {
        if let Some(st) = ex.stacktrace.as_mut() {
            for frame in st.frames.iter_mut() {
                frame.vars.clear();
            }
        }
    }
    for thread in event.threads.iter_mut() {
        if let Some(st) = thread.stacktrace.as_mut() {
            for frame in st.frames.iter_mut() {
                frame.vars.clear();
            }
        }
    }

    event
}

/// Convenience-Wrapper. No-Op wenn Sentry nicht init oder Consent aus.
#[allow(dead_code)]
pub fn capture_message(message: &str, level: sentry::Level) {
    sentry::capture_message(message, level);
}

/// Convenience-Wrapper. No-Op wenn Sentry nicht init oder Consent aus.
#[allow(dead_code)]
pub fn capture_error<E: std::error::Error + ?Sized>(err: &E) {
    sentry::capture_error(err);
}
