// v0.9.0 (#GlitchTip) — Sentry-Init fuer das Tauri-Frontend (React).
//
// Spec: docs/spec/v0.9.0-glitchtip-self-hosted.md
//   + docs/spec/v0.9.0-telemetry-contract.md Sektion 9 (Datenschutz-Gates)
//
// Anders als Recorder/Webapp ist der Client ein Pilot-facing Tool
// → Opt-In ist Pflicht (DSGVO Art. 6 (1) a). Default = aus.
//
// DSN: Build-time via Vite-env VITE_SENTRY_DSN_CLIENT.
// Consent: localStorage `aeroacars.errorReporting.enabled` ("true"|"false")
//   + Mirror in den Rust-Backend via Tauri-Command `error_reporting_set_consent`.
//
// Privacy:
//   - beforeSend strippt nicht-allowlisted Tags, User-PII, Request-Daten
//   - Wenn !consent → Event verworfen (Defense-in-Depth: Rust hat eigene Atomic-Gate)

import * as Sentry from "@sentry/react";

export const STORAGE_KEY = "aeroacars.errorReporting.enabled";

const ALLOWED_TAG_KEYS: ReadonlySet<string> = new Set([
  "app.component",
  "app.version",
  "os",
  "os.version",
  "simulator",
  "aircraft",
  "airport",
  "runway",
  "pirep.id",
  "callsign",
  "route",
  "phase",
  "pilot.hash",
  "forensics.version",
  "error.code",
  "error.status_code",
  "error.kind",
  "feature.flag",
  "distance.bucket",
]);

export function getConsent(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

/**
 * Pilot hat noch nie eine Entscheidung getroffen.
 * Wird vom First-Run-Banner abgefragt.
 */
export function consentIsUnset(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === null;
  } catch {
    return true;
  }
}

export function setConsent(enabled: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, enabled ? "true" : "false");
  } catch {
    // ignore
  }
}

/**
 * Initialisiert Sentry. No-Op wenn DSN nicht in build-time-env gesetzt war.
 * MUSS sehr frueh in main.tsx aufgerufen werden.
 */
export function initSentry(appVersion: string): boolean {
  const dsn = (import.meta.env.VITE_SENTRY_DSN_CLIENT as string | undefined)?.trim();
  if (!dsn) {
    // Kein Log noisy machen — viele Builds laufen ohne DSN (Dev, contributors).
    return false;
  }

  Sentry.init({
    dsn,
    release: `aeroacars-client@${appVersion}`,
    environment: import.meta.env.PROD ? "production" : "development",
    sendDefaultPii: false,
    tracesSampleRate: 0,
    integrations: [],
    beforeSend(event) {
      // Consent-Gate. Wird live aus localStorage gelesen, damit Toggle sofort wirkt.
      if (!getConsent()) return null;
      try {
        return redactEvent(event);
      } catch (err) {
        console.warn("[sentry] redactEvent failed, dropping:", err);
        return null;
      }
    },
    beforeBreadcrumb(crumb) {
      if (crumb.category === "fetch" || crumb.category === "xhr") {
        if (crumb.data?.url && typeof crumb.data.url === "string") {
          crumb.data.url = crumb.data.url.split("?")[0];
        }
      }
      return crumb;
    },
    initialScope: {
      tags: {
        "app.component": "client",
        "app.version": appVersion,
      },
    },
  });
  return true;
}

export function redactEvent(event: Sentry.ErrorEvent): Sentry.ErrorEvent {
  if (event.tags) {
    const cleanTags: Record<string, string | number | boolean> = {};
    for (const [key, value] of Object.entries(event.tags)) {
      if (ALLOWED_TAG_KEYS.has(key) && value != null) {
        cleanTags[key] = value as string | number | boolean;
      }
    }
    event.tags = cleanTags;
  }

  if (event.user) {
    event.user = {
      id: event.user.id && /^[a-f0-9]{1,16}$/i.test(String(event.user.id))
        ? event.user.id
        : undefined,
      email: undefined,
      username: undefined,
      ip_address: undefined,
    };
  }

  if (event.request) {
    event.request = {
      method: event.request.method,
      url: event.request.url?.split("?")[0],
    };
  }

  if (event.breadcrumbs) {
    event.breadcrumbs = event.breadcrumbs.map((crumb) => {
      if (crumb.data) {
        const cleanData: Record<string, unknown> = {};
        for (const [k, v] of Object.entries(crumb.data)) {
          const kl = k.toLowerCase();
          if (
            kl.includes("token") ||
            kl.includes("password") ||
            kl.includes("authorization") ||
            kl.includes("cookie") ||
            kl === "body" ||
            kl === "request_body"
          ) {
            cleanData[k] = "[REDACTED]";
          } else {
            cleanData[k] = v;
          }
        }
        return { ...crumb, data: cleanData };
      }
      return crumb;
    });
  }

  if (event.exception?.values) {
    for (const ex of event.exception.values) {
      if (ex.stacktrace?.frames) {
        for (const frame of ex.stacktrace.frames) {
          delete frame.vars;
        }
      }
    }
  }

  return event;
}

export { Sentry };
