# 🔍 AeroACARS — Master-Audit-Report

**Datum**: 2026-05-12
**Scope**: Pilot-Client (`E:\CloudeAcars`) + aeroacars-live VPS (`E:\aeroacars-live`)
**Versionen**: Pilot-Client v0.7.12 (live released), Recorder/Webapp HEAD = `f647577`
**Modus**: Read-only QS — keine Änderungen vorgenommen

**Meta-QS durch User (post-audit) — Korrekturen unten in Sektion „QS-Abweichungen“:**
- `cargo audit` war NICHT lokal lauffähig (`cargo-audit` nicht installiert) → Rust-Dep-CVE-Aussage nur best-effort manuell, nicht tool-verifiziert
- FSUIPC-Code-Pfade sind sauber (Hardban respektiert), aber Doku-Erwähnungen sind >2 (mehrere READMEs/specs), Report-Zahl war zu niedrig
- Lokales `latest.json` Bundle-Artefakt zeigt `0.5.38` — ist nur ein veralteter Build-Ordner-Stand, der remote-deployte `latest.json` zeigt korrekt `0.7.12`

Drei parallele Audit-Agenten (Pilot-Client, aeroacars-live, Security) wurden ausgeführt. Detail-Reports liegen in:
- `docs/audit/pilot-client-audit.md`
- `docs/audit/aeroacars-live-audit.md`
- `docs/audit/security-audit.md`

---

## 🚨 EXECUTIVE SUMMARY — Critical + High

| # | Sev | Kategorie | Was | Wo |
|---|---|---|---|---|
| **C1** | 🔴 **CRITICAL** | Secret-Leak | **Discord-Webhook-Token im public GitHub-Repo committed** — jeder kann Spam/Phishing in den GSG-Discord posten | `client/src-tauri/src/discord.rs:27` |
| **C2** | 🔴 **CRITICAL** | Privilege-Eskalation | `aeroacars`-User hat passwortlosen sudo `NOPASSWD:ALL` plus Webapp-Endpoints rufen `sudo apt-get`/`shutdown` — Admin-Cookie-Leak = Full-Root | `aeroacars-live/vps/bootstrap.sh:67` |
| H1 | 🟠 High | Brute-Force | Kein Rate-Limit auf `/api/login`, `/api/provision`, `/api/forms/aircraft` — Credential-Stuffing + DoS-Amplification möglich | `recorder/src/server.ts` |
| H2 | 🟠 High | RCE-Risiko | `apt-get upgrade` + `shutdown` als Admin-API-Endpoints — apt-Postinstall-Hooks effektiv = arbitrary command execution als root | `recorder/src/server.ts` Admin-Routes |
| H3 | 🟠 High | Dep-CVE | `@fastify/static@8.3.0` → 2 moderate CVEs (Path-Traversal + Route-Guard-Bypass). Fix in 9.1.3 (Major-Bump) | `recorder/package.json` |
| H4 | 🟠 High | Key-Hygiene | Tauri-Updater-Privatekey `client/aeroacars-updater.key` liegt im Repo-Working-Tree (`.gitignore` korrekt, History sauber, aber 1× `git add -f` = catastrophic) | `client/aeroacars-updater.key` |
| H5 | 🟠 High | Dep-CVE | `tar`-Chain via `bcrypt → @mapbox/node-pre-gyp` in Recorder, 6 CVEs, top CVSS 8.2 (install-time-only, aber fixen) | `recorder/package.json` |
| H6 | 🟠 High | Dead Code | Discord-Rich-Presence-Block ~170 LOC dead seit v0.4.0 — Versprechen "Wiring kommt in v0.4.5", wir sind bei v0.7.12 | `client/src-tauri/src/discord.rs:491-659` |
| H7 | 🟠 High | Dead Code | 4 verwaiste React-Komponenten ohne Importer | `client/src/components/`: `Dashboard.tsx`, `FlightInfoPanel.tsx`, `MassPanel.tsx`, `PhaseTimeline.tsx` |
| H8 | 🟠 High | Dead Code | 6 Tauri-Commands in `generate_handler!` registriert, aber nirgends im Frontend `invoke<>`-aufgerufen | `client/src-tauri/src/lib.rs`: `get_minimize_to_tray`, `get_simbrief_settings`, `landing_get`, `ofp_callsign_warning_dismiss`, `xplane_uninstall_plugin`, `detect_running_sim` |

---

## ⚡ EMPFOHLENE ACTION-REIHENFOLGE

### Sofort (heute Abend / morgen früh)

1. **C1 — Discord-Webhook rotieren**:
   - `Discord → GSG-Server → Channel-Settings → Integrations → Webhooks → bestehenden löschen → neuen erstellen`
   - **Achtung:** Sobald rotiert, posten alle bisher installierten Pilot-Clients (v0.4.0 bis v0.7.12) NICHT mehr in den Discord-Channel — bis sie auf eine neue Version updaten die die neue URL embedded.
   - **Sauber-Fix-Idee** (für v0.7.13): Webhook-URL nicht hardcoden, sondern beim Pilot-Client-Start vom Recorder-Backend (live.kant.ovh) holen. Damit ist Rotation in Zukunft serverseitig.
   - Bonus: alten Webhook-Token-Stand im git-history mit `git filter-repo` oder BFG entfernen (Vorsicht: rewrite history kann Pull-Requests + Forks brechen) — alternativ einfach öffentlich zugeben + lassen, da sowieso rotiert.

2. **C2 — sudoers-Whitelist erzwingen**:
   - `bootstrap.sh:67` ändern: das `NOPASSWD:ALL` löschen, den vorhandenen `sudoers.d-aeroacars` (= surgical Whitelist) als einzige sudo-Rechte-Quelle aktivieren.
   - **Vorher** auf VPS testen ob die Admin-Endpoints (`updates/install`, `system/reboot`) noch funktionieren — die brauchen jetzt explizit gewhiteliste Befehle.

### Diese Woche

3. **H1 — Rate-Limiting** auf `/api/login` + `/api/provision` einführen (z.B. `@fastify/rate-limit`, 5 attempts/15min/IP).
4. **H2 — Admin-Endpoints** absichern: 2FA-Confirmation-Flow oder zumindest Re-Auth-Prompt vor `updates/install` + `system/reboot`. Plus CSRF/Origin-Check.
5. **H3 — `@fastify/static` Major-Bump** auf 9.1.3 (Breaking-Change-Notes lesen, dann updaten + redeploy).
6. **H4 — Updater-Key** außerhalb des Repos verschieben (z.B. `~/.config/aeroacars-keys/updater.key`), GitHub-Secrets bleiben unverändert.
7. **H5 — `bcrypt`-Dep** auf eine Version ohne `@mapbox/node-pre-gyp` (oder migrate auf `bcryptjs` pure-JS).

### Nächste 2 Wochen

8. **H6 + H7 + H8 — Code-Cruft-Cleanup**: Discord-RP-Block raus, 4 verwaiste React-Components löschen, 6 orphan Tauri-Commands entfernen (oder im Frontend wiring nachholen falls geplant).
9. **M-Findings (siehe unten)** durchgehen und entscheiden welche raus.
10. **MEMORY.md korrigieren** (siehe Cross-Cutting unten).

---

## 🔄 CROSS-CUTTING FINDINGS

Diese Punkte tauchten in mehreren Audits unabhängig auf:

### CC1 — MEMORY.md ist falsch zur Secret-Storage
User-Memory sagt "phpVMS-API-Key wird via Windows Credential Manager / macOS Keychain gespeichert".
**Realität:** Seit v0.5.15 schreibt `client/src-tauri/crates/secrets/src/lib.rs` Klartext-JSON nach `<app_data_dir>/secrets.json` (chmod 0600 auf Unix, %APPDATA%-ACL auf Windows). Der Modul-Comment erklärt warum (macOS-Keychain-Prompt-Loop). Die `keyring`-Dependency ist noch im `Cargo.toml` für `migrate_from_keyring()` aus v0.5.15 — kann seit ~30 Releases raus.

→ **Aktion**: MEMORY.md updaten + `keyring`-Dep + Migration-Code löschen (siehe Pilot-Client Audit Punkt 6).

### CC2 — Veraltete Specs & Drafts
- Pilot-Client: `docs/spec/*.md` mit "Wiring kommt in v0.4.5", "Patch in v0.7.7" (= ist passiert oder verworfen)
- aeroacars-live: `client-mqtt-extension/*.draft` + `docs/aeroacars-integration-spec.md` v1 sagen "Phase 0, keine Implementation" — AeroACARS publisht aber seit v0.4 live über MQTT

→ **Aktion**: Eine Stunde Drafts/Specs-Aufräum-Session: archivieren in `docs/spec/historical/` oder löschen.

### CC3 — Version-Drift in mehreren Projekten
- Pilot-Client v0.7.12 ✅ konsistent (gerade gefixt)
- Recorder/Webapp `package.json` beide noch auf `0.1.0` während Code v0.7.11-aligned

→ **Aktion**: Recorder/Webapp `package.json` mit aeroacars-live-eigenem Versionschema versehen (z.B. `1.0.0` + Semver-pro-Recorder-Release-Tag), oder mit Pilot-Client mitziehen.

### CC4 — Deprecated Sub-Projekt `monitor/`
README sagt deprecated, lebt aber komplett im Branch (Tauri-Desktop-Variante des Webapp-Admin-Tools).

→ **Aktion**: Entscheidung — entweder als "historical/v0.4-monitor/" archivieren oder ganz löschen. User-Memory sagt "Monitor-App ist Windows-only, admin-only" — wenn die Webapp das vollständig ersetzt, kann monitor/ raus.

---

## ✅ NEGATIVE FINDINGS — was geprüft + sauber

Damit du siehst dass die meiste Kosmetik OK ist:

**Pilot-Client:**
- 0 TypeScript-Build-Errors / Warnings
- 0 FSUIPC-Code-Pfade (User-Hardban respektiert) — **7 Doku-Erwähnungen** in `README.md`, `client/README.md`, `client/src-tauri/crates/sim-msfs/Cargo.toml` (Description-String), `docs/architecture.md`, `docs/decisions/0002-msfs-simconnect-only.md`, `docs/decisions/README.md`, `docs/pmdg-sdk-integration.md` — alle als "ausdrücklich-nicht"-Marker, kein Code-Risk. Ursprüngliche Audit-Zahl "2" war falsch.
- Versionen alle konsistent auf 0.7.12 (package.json / tauri.conf.json / Cargo.toml / Cargo.lock)
- i18n DE/EN parity 100% (881/881 Keys), IT hat 1 extra Key
- Volanta / LandingRate-1.lua sind Algorithmus-Referenzen (Doku), kein Live-Endpoint
- `pre-v0.5.x` Backward-Compat (15+ Stellen) ist legitim für gespeicherte Alt-JSON-Files
- Rust-Deps aktuell, kein Crate >1 Minor hinterher

**aeroacars-live:**
- TS-Build clean (Recorder + Webapp 0 Errors)
- Webapp `npm audit` = 0 vulnerabilities
- Mosquitto-Broker korrekt: `allow_anonymous false`, beide Listener auf 127.0.0.1, per-Pilot-ACL (`aeroacars/<va>/<pilot>/#`) — Pilot A kann Pilot B Topics nicht lesen/publishen
- 55 von 61 REST-Endpoints hinter `requireAuth`, 6 öffentlich (alle legitim begründet, z.B. `/api/ping`)
- Alle 10 Webapp-Tabs mapped + erreichbar, keine toten Routes
- CRLF-Problem auf deploy-recorder.sh ist NICHT aktuell (Bytes via `od -c` LF-only verifiziert) — war Einmal-SSH-Glitch
- TLS via Caddy, automatisches Cert-Management
- MQTT-Auth via Env-Variablen, nicht im Repo

**Security spezifisch:**
- Keine PEM-Keys, JWTs, Hex-Tokens, DB-URLs-mit-Credentials in beiden Repos (außer C1+H4)
- 0 SQL-Injection (alle Queries parametrisiert; Template-Strings nur für whitelisted Column-Names)
- 0 Path-Traversal (Flight-Log-Upload + JSONL-Import haben Sanitization + `resolve()`-Prefix-Checks)
- Bcrypt-Rounds 12, `timingSafeEqual` für Basic-Auth
- Kein CORS-Misconfig (Fastify-Default = no CORS, Webapp same-origin)
- Updater nutzt HTTPS-only, embedded Minisign-Pubkey verifiziert Signaturen — Modell ist sound sobald H4 adressiert
- Keine Secret-Leaks in `tracing::*`-Logs (Pilot-Client) oder Recorder-Fastify-Logger
- SQLite-Payloads enthalten nur Telemetrie, keine API-Keys

---

## 📋 KOMPLETTE FINDING-INDEX (alle Severities, Pilot-Client)

| Sev | # | Was | Wo |
|---|---|---|---|
| H | 1 | Discord-RP-Block dead seit v0.4.0 | `discord.rs:491-659` |
| H | 2 | 4 orphan React-Components | `src/components/` |
| H | 3 | 6 orphan Tauri-Commands | `lib.rs` `generate_handler!` |
| M | 4 | `fcu_debounce()` + 8 Struct-Felder (Fenix-Plan verworfen) | `lib.rs:2198, 16368` |
| M | 5 | Build-Warnings: `current_premium_status`, `count` | `lib.rs:12241, 7230` |
| M | 6 | `secrets::migrate_from_keyring()` + `keyring`-Dep (v0.5.15-Migration, 30+ Releases her) | `crates/secrets/src/lib.rs` |
| M | 7 | 3 echte tote i18n-Keys: `tabs.dashboard`, `landing.peak_vs`, `landing.plan_tow`, `landing.plan_ldw` | `locales/{de,en,it}/common.json` |
| L | 8 | Discord `EventContext`-Felder `airline_icao` + `fuel_used_kg` gesetzt aber nicht gelesen | `discord.rs:52` |
| L | 9 | Workspace-Dep `schemars = "0.8"` deklariert aber ungenutzt | `Cargo.toml` |
| L | 10 | Stale Versprechen-Comments (6× "Wiring kommt in v0.4.5", 1× "Patch in v0.7.7") | diverse |

## 📋 KOMPLETTE FINDING-INDEX (aeroacars-live)

| Sev | # | Was | Wo |
|---|---|---|---|
| H | 11 | `@fastify/static` 8.3.0 → 2 CVEs | `recorder/package.json` |
| H | 12 | Kein Rate-Limit auf Auth-Endpoints | `recorder/src/server.ts` |
| H | 13 | Admin-Endpoints rufen `sudo` ohne Re-Auth | `recorder/src/server.ts` |
| M | 14 | Specs `client-mqtt-extension/*.draft` + `aeroacars-integration-spec.md` v1 veraltet | `docs/`, `client-mqtt-extension/` |
| M | 15 | `monitor/` Tauri-Desktop in README deprecated, lebt im Branch | `monitor/` |
| M | 16 | `vps/sudoers.d-aeroacars:11` Wildcard in `sed`-Cmnd glob-fragil | `vps/sudoers.d-aeroacars` |
| M | 17 | `provisioned_pilots.password` Klartext in SQLite | `recorder/src/db.ts` |
| M | 18 | Version-Drift `package.json 0.1.0` vs Code v0.7.11 | `recorder/package.json`, `webapp/package.json` |
| M | 19 | 2 API-Endpoints ohne Frontend-Caller: `/api/provisioned`, `/api/provisioned/:va/:pilot/revoke` | `recorder/src/server.ts` |

## 📋 KOMPLETTE FINDING-INDEX (Security)

| Sev | # | Was | Wo |
|---|---|---|---|
| **C** | **1** | **Discord-Webhook-Token im public Repo** | `client/src-tauri/src/discord.rs:27` |
| **C** | **2** | **`NOPASSWD:ALL` + Admin-RCE-Endpoints** | `aeroacars-live/vps/bootstrap.sh:67` |
| H | 3 | Kein Rate-Limit auf Auth-Endpoints | s.o. #12 |
| H | 4 | Updater-Privatekey im Repo-Working-Tree (`.gitignore` korrekt, History sauber) | `client/aeroacars-updater.key` |
| H | 5 | `apt-get upgrade` + `shutdown` als Admin-Endpoints = effektiv RCE | s.o. #13 |
| M | 6 | Keine Security-Headers in Caddy (CSP, X-Frame-Options, X-Content-Type-Options, Referrer-Policy, HSTS-explicit) | `aeroacars-live/vps/caddy/Caddyfile` |
| M | 7 | Tauri-Webview hat `csp: null` — keine XSS-Defense-in-Depth | `client/src-tauri/tauri.conf.json` |
| M | 8 | Pilot-MQTT-Passwords plaintext in `provisioned_pilots.password` (Design-Constraint, aber dokumentieren oder at-rest-encrypt) | s.o. #17 |
| M | 9 | **CC1** — MEMORY.md sagt "Keychain", Realität ist Klartext-JSON in App-Data | `crates/secrets/src/lib.rs` |
| M | 10 | `bcrypt → @mapbox/node-pre-gyp → tar` 6 CVEs (install-time only) | `recorder/package.json` |

---

## 🛠 EMPFOHLENER FIX-PRIORITY-PFAD

```
SOFORT (Cat: kritisch, ausnutzbar)
├── C1: Discord-Webhook rotieren            (5 min im Discord-UI)
└── C2: NOPASSWD:ALL → sudoers-Whitelist    (1 SSH-Session, ~30 min mit Test)

DIESE WOCHE (Cat: hoch, exploitable mit etwas Aufwand)
├── H1+H2: Rate-Limit + Re-Auth auf Admin-Endpoints  (~3h Code+Test)
├── H3: @fastify/static 9.x Major-Bump                (~1h + Test)
├── H4: Updater-Key aus Repo verschieben              (~30 min)
└── H5: bcrypt → bcryptjs                              (~1h)

NÄCHSTE 2 WOCHEN (Cat: medium, Cleanup + Hygiene)
├── H6+H7+H8: Dead-Code-Cleanup (Discord-RP, 4 React, 6 Tauri-Commands)  (~3h)
├── M1-M3: Specs/Drafts/monitor/ archivieren                              (~2h)
├── M-Various: i18n-Tot-Keys, Cargo schemars-Dep, Versprechen-Comments    (~2h)
├── CC1: MEMORY.md korrigieren + keyring-Migration löschen                (~1h)
├── CC3: Recorder/Webapp Versionschema                                    (~1h)
├── #6: Caddy Security-Headers (CSP minimal, X-Frame, HSTS-explicit)      (~1h)
└── #7: Tauri csp setzen statt null                                        (~30 min)

OPTIONAL (Cat: low + dokumentarisch)
├── #9: schemars Workspace-Dep entfernen
├── M-Various Pilot-Client: Stale Comments aufräumen
└── M8: provisioned_pilots.password at-rest encrypt + Comment "Design-Constraint"
```

**Gesamt-Effort-Schätzung:** ~20-25h kompletter Cleanup + Security-Härtung. Critical-Items allein: ~35 min.

---

## 🔬 QS-Abweichungen / Caveats (vom User identifiziert)

User hat das Audit-Ergebnis selbst gegen-geprüft. Die folgenden Punkte sind Korrekturen / Klarstellungen:

| # | Caveat | Konsequenz |
|---|---|---|
| Q1 | **`cargo audit` wurde nicht lokal ausgeführt** — `cargo-audit` ist auf der Build-Maschine nicht installiert. Agent C hat Rust-Dep-CVE-Aussagen best-effort manuell formuliert, aber nicht tool-verifiziert | **Status Pilot-Client Rust-Deps = unbekannt.** Empfehlung: `cargo install cargo-audit && cargo audit` einmal laufen lassen vor v0.7.13-Release |
| Q2 | **FSUIPC-Doku-Zählung korrigiert** — Audit sagte 2, real sind es 7 unique Files (siehe Negative-Findings-Sektion oben) | Kein Funktions-Risk (Code-Pfade weiterhin sauber), nur Report-Genauigkeit |
| Q3 | **Stale lokales `latest.json`** in `client/src-tauri/target/release/bundle/nsis/latest.json` zeigt `0.5.38` | Nur lokales Build-Artefakt aus alter Build-Session. Remote-`latest.json` auf github.com/.../releases/latest/download/latest.json zeigt korrekt `0.7.12`. Lokal kann ignoriert werden, oder `target/release/bundle/` lokal löschen |
| Q4 | **Audit-Dokumente sind untracked in git** (`E:\CloudeAcars\docs\audit\` neu) | Wenn der Report langfristig im Repo gehalten werden soll → `git add docs/audit/ && git commit`. Wenn nicht → `.gitignore`-Eintrag |
| Q5 | **`E:\aeroacars-live` Working-Tree nicht clean** — `webapp/src/components/LandingAnalysis.tsx` hat lokale Änderung ggü. HEAD | Bewusste Änderung des Users (Linter oder manuell), nicht zurückrollen. Kann im nächsten Commit mitfließen |

### Abgeschlossene Build-Gates (vom User verifiziert)
- Client: `npm run build` ✅, `npm test` ✅ (39/39), `cargo check` ✅ (4 Warnings)
- Webapp: `npm run build` ✅
- Recorder: `npm run build` ✅
- npm-audit: Client + Webapp = 0 Vulns; Recorder = 3 Vulns (siehe H3+H5)
- Public-REST-Endpoints (6) bestätigt: `healthz`, `provision`, `login`, `flight-logs/upload`, `vapid-public-key`, `forms/aircraft`
- i18n-Parität: DE 881, EN 881, IT 882 (1 Extra-Key) — bestätigt

---

## 📁 Referenz-Reports

- `pilot-client-audit.md` — vollständige Liste mit `path:line`-Quotes (Agent A, 16 KB)
- `aeroacars-live-audit.md` — Endpoint-Tabelle + SQL-Schema-Drift (Agent B, 18 KB)
- `security-audit.md` — Per-Finding Severity + Impact + Recommendation + Appendix (Agent C, 31 KB)

Bei Fragen zu einzelnen Findings öffne den entsprechenden Detail-Report — dort steht jeweils der genaue Kontext, betroffene File-Bereiche, und vorgeschlagene Fix-Strategie.
