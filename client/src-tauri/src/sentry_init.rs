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
        s.insert("route");
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

/// Initialisiert Sentry. No-Op wenn DSN nicht in env zur Build-Zeit gesetzt war.
///
/// MUSS frueh in `lib.rs::run()` aufgerufen werden, vor dem Tauri-Builder,
/// damit auch Bootstrap-Panics gefangen werden.
pub fn init() {
    SENTRY_GUARD.get_or_init(|| {
        // build.rs setzt AEROACARS_SENTRY_DSN als compile-time env.
        // Wenn beim Build leer → option_env! gibt None → kein Init.
        let dsn = option_env!("AEROACARS_SENTRY_DSN").unwrap_or("").trim();
        if dsn.is_empty() {
            tracing::info!("[sentry] no DSN configured (AEROACARS_SENTRY_DSN at build-time); skipping init");
            return None;
        }

        let release = format!("aeroacars-client@{}", env!("CARGO_PKG_VERSION"));
        let environment = if cfg!(debug_assertions) {
            "development"
        } else {
            "production"
        };

        let guard = sentry::init(sentry::ClientOptions {
            dsn: dsn.parse().ok(),
            release: Some(release.clone().into()),
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
        });

        // Initial-Scope: Komponenten-Tags
        sentry::configure_scope(|scope| {
            scope.set_tag("app.component", "client");
            scope.set_tag("app.version", env!("CARGO_PKG_VERSION"));
            scope.set_tag("os", std::env::consts::OS);
        });

        tracing::info!("[sentry] initialized for release {}", release);
        Some(guard)
    });
}

/// Setzt den Pilot-Consent. `true` = Events duerfen raus, `false` = alles wird
/// im before_send-Hook verworfen. Wird vom Tauri-Command + Settings-UI gerufen.
pub fn set_consent(enabled: bool) {
    CONSENT.store(enabled, Ordering::Relaxed);
    tracing::info!("[sentry] consent set to {}", enabled);
    // Falls dem Pilot die Erinnerung kommt und er ausschaltet: pending events flushen
    // (= sicherstellen dass NICHTS mehr danach geht). Ein flush-then-close ist nicht
    // moeglich ohne re-init, also reicht es die Atomic auf false zu setzen — alle
    // weiteren Events landen im before_send → None.
    if !enabled {
        // Optional: laufende send-tasks abwarten
        let _ = sentry::Hub::current().client().map(|c| c.flush(Some(std::time::Duration::from_secs(2))));
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
