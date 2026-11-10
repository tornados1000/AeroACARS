# aeroacars-live — Read-Only QS-Audit

**Audit-Datum:** 2026-05-12
**Branch:** `claude/aeroacars-windows-app-6lPsp` (HEAD `f647577`)
**Scope:** recorder (Node/Fastify), webapp (React/Vite), vps/, docs/, client-mqtt-extension/
**Methode:** statisch + `tsc --noEmit` + `npm outdated/audit` (lesend)

Severity: **Critical** / **High** / **Medium** / **Low** / **Info**.
`repo:path:line` ist relativ zu `E:/aeroacars-live/`.

---

## 1. Dead / Unused TypeScript-Code

`tsc --noEmit` lief für recorder und webapp ohne Output (= 0 Fehler, 0 Warnungen). Keine Unused-Imports im Strict-Sinn.

- **Low** — `recorder/src/db.ts:1804` `listTouchdowns(limit)` wird nirgends aufgerufen (ersetzt durch `listTouchdownsWithAircraft` an server.ts:202). Empfehlung: löschen oder `@deprecated` markieren.
- **Low** — `recorder/src/db.ts:1947` `listPireps(limit)` analog ungenutzt (ersetzt durch `listPirepsWithAircraft` an server.ts:315). Empfehlung: löschen.
- **Info** — `recorder/src/db.ts:30` `interface AdminUser` exportiert aber nur über `getAdmin()`-Rückgabewert benutzt — könnte intern bleiben. Kein Action-Item.
- **Info** — Im Webapp/Recorder kein toter Code laut TypeScript-Strict-Mode. `recoderRow` Helper an server.ts:1383/1393 sind beide verlinkt.

---

## 2. Version-Drift

- **Medium** — Beide package.json behaupten `"version": "0.1.0"` (`recorder/package.json:3`, `webapp/package.json:4`), während Repo + Pilot-Client bei v0.7.11 stehen. CHANGELOG / Release-Tags leben nur in Git. Empfehlung: entweder semver synchron zu Pilot-Client führen oder einen `RECORDER_VERSION`-String aus `index.ts` an `/api/healthz` exposen damit Deploys verifizierbar sind.
- **Info** — Webapp-Code referenziert in Kommentaren v0.5.18, v0.5.23, v0.5.25, v0.5.26, v0.5.34, v0.5.49, v0.7.6, v0.7.7, v0.7.11. Die meisten beschreiben historisches Schema-Versioning + sind sinnvoll. Keine widersprüchlichen v0.5.x-Behauptungen gefunden.
- **Low** — `recorder/src/server.ts:60,74` Kommentar "v0.5.x: Diagnostics-Tab benötigt …" und `cacheControl`-Block sind nicht mehr "x" sondern stabil — Marker veraltet, harmlos.

---

## 3. Deprecated / Stale Features

- **Medium** — `monitor/` (Tauri-Desktop-App) ist offiziell deprecated (README.md:36, monitor/README.md:1) — bleibt aber im Repo + wird vom `tsc`-Scope nicht abgedeckt. Da Memory-Constraint Windows-only/admin-only Monitor lautet, ist die Tauri-App **definitiv** zu archivieren oder zumindest auf `.gitattributes export-ignore` zu stellen. Empfehlung: in eigenen Branch verschieben + aus main entfernen.
- **Low** — `webapp/src/data/phaseColors.ts:23` Phase `ON_BLOCK` ist explizit "legacy alias — Pre-v0.5.18 Clients". Keep solange Alt-Daten in DB existieren, aber Tracking-Comment ergänzen wann safely entfernbar.
- **Low** — `_ApproachStabilityCard.tsx:18` `hasV2`-Check toleriert legacy-Pre-v0.5.25-Touchdowns. Identische Logik in `_LandingQualityCard.tsx`. OK so, kein Aufräumen nötig.
- **Info** — Unterstrich-Prefix (`_ApproachStabilityCard`, `_ApproachChart`, `_LandingQualityCard`) ist **kein** Deprecated-Marker — diese Files werden von `LandingAnalysis.tsx` importiert (siehe Grep). Convention scheint: "Sub-Component, nur von einer Parent-Component verwendet". Empfehlung: in CONTRIBUTING / README dokumentieren oder die Files in `components/landing/` einsortieren.

---

## 4. Stale Specs / Draft-Files

- **Medium** — `docs/aeroacars-integration-spec.md` (v1 manueller Token-Flow) ist von `docs/aeroacars-integration-spec-v2.md` (Auto-Provisioning) abgelöst — der v1-Spec referenziert "Live-Monitor-Desktop-App" als Zielgruppe, was nicht mehr stimmt (Monitor deprecated). Empfehlung: v1 in `docs/archive/` verschieben oder mit Deprecated-Banner versehen.
- **Medium** — `client-mqtt-extension/{Cargo.toml.draft, lib.rs.draft, README.md}` referenzieren explizit Phase-0/"Keine Implementierung". Realität: AeroACARS-Pilot-Client publisht seit v0.4+ live MQTT (siehe Commits im Hauptrepo). Empfehlung: gesamte `client-mqtt-extension/` löschen oder durch Verweis auf Hauptrepo-Crate ersetzen.
- **Low** — `docs/topic-schema.md` ist aktuell (referenziert `block`/`takeoff`-Channels aus v0.5.14+) — keep.
- **Info** — `docs/architecture.md`, `docs/auth-model.md` haben kein Datum / Version. Empfehlung: Frontmatter `last-updated: YYYY-MM-DD`.

---

## 5. API-Endpoints-Audit

**Endpoints in `recorder/src/server.ts` (61 Routen + WS `/api/live`):** Alle außer 6 mit `preHandler: requireAuth`.

### 5a. Ohne `requireAuth` (begründet oder problematisch)

| Endpoint | Begründung | Severity |
|---|---|---|
| `GET /api/healthz` (`server.ts:108`) | öffentlicher Health-Probe | OK |
| `POST /api/provision` (`server.ts:112`) | Auto-Provisioning, validiert phpVMS-API-Key in `provision.ts` | Medium — siehe §9 |
| `POST /api/login` (`server.ts:132`) | Login, bcrypt | Medium — kein Rate-Limit (§9) |
| `POST /api/flight-logs/upload` (`server.ts:578`) | HTTP-Basic gegen `provisioned_pilots` + timing-safe compare | OK |
| `GET /api/admin/push/vapid-public-key` (`server.ts:1012`) | Public-Key, ungefährlich | OK |
| `POST /api/forms/aircraft` (`server.ts:1122`) | Validiert `X-Forms-Token` shared secret aus DB | OK |

### 5b. Endpoints WITHOUT Webapp-Caller (server hat sie, webapp nutzt sie nicht)

- **Info** — `GET /api/provisioned` und `POST /api/provisioned/:va/:pilot/revoke` (`server.ts:121, 123`) — keine Aufrufer in `webapp/src/`. Es gibt einen `db.listProvisionedPilots()`-Backend-Pfad, aber kein UI-Element. Empfehlung: Webapp-Admin-View bauen ODER Endpoints entfernen.
- **Info** — `GET /api/touchdowns/forensik` (`server.ts:707`) → `api.touchdownForensik` an `api.ts:230`, aufgerufen in `Touchdowns.tsx:272`. **OK.**
- **Info** — Alle übrigen Endpoints sind via `webapp/src/api.ts` oder direkt via `fetch(...)` in den Tabs aufgerufen (`/api/admin/jsonl-files`, `/api/admin/jsonl-import`, `/api/pireps/:id/jsonl`, `/api/pireps/:id/export`, `/api/flights/:va/:pilot/export`, `/api/admin/push/*` — alle wiedergefunden).

### 5c. Auth-Modell-Hinweis

- **Info** — `requireAuth` (`server.ts:1373`) authentifiziert **NUR** das Admin-Cookie; nimmt **nicht** die VA/Pilot-Params in den URLs in den Filter. Das ist beabsichtigt (Admin-only Tool), aber dokumentationswürdig: jeder Admin sieht alle Piloten aller VAs. Siehe §9.

---

## 6. SQL-Schema vs Code-Drift

- **Low** — `positions`-Spalten `simulation_rate`, `paused`, `autopilot_master`, `light_beacon`, `light_strobe`, `light_landing` (`db.ts:174-203`) werden in CLI-Tools + importer geschrieben aber **nirgends gelesen** im Webapp (Grep: 0 Treffer in `webapp/src`). Sind Telemetrie-Forensik-only — OK, aber pro Spalte ~9 Bytes pro Position × ~30d Retention = signifikant. Empfehlung: zumindest dokumentieren ob bewusst archiviert.
- **Info** — `flight_session_stats` (`db.ts:207`) hat 23 Spalten, alle werden in `recomputeSessionStats()` + telemetry-Endpoints gelesen. Keine Toten.
- **Low** — Kein Index auf `positions(ts)` allein — alle Queries gehen über `(va_prefix, pilot_id, ts)` (covered by `idx_positions_pilot_ts`). Bei Cross-Pilot-Heatmap-Queries (z.B. `allTouchdownsForHeatmap`) hilft das nicht, aber die laufen über `touchdowns.ts`-Index. **OK.**
- **Low** — `pireps.payload_json` wird in `searchPireps` (server.ts:330) via volltext-Search (`q.q`-Param) gefiltert — aber kein FTS-Index. Bei <2000 PIREPs/Jahr ungefährlich, ab >10k merklich.
- **Info** — `touchdowns` hat seit v0.5.34 `UNIQUE(va_prefix, pilot_id, ts)` (`db.ts:557`) — robust gegen QoS-1 Re-Delivery. Migration ist defensiv geschrieben.

---

## 7. Webapp Routes

TabKeys aus `useHashRoute.ts:21`: `live, flights, landings, reports, pilots, history, trends, heatmap, system, admin`. Alle 10 sind in `App.tsx:118-130` einer Component zugeordnet. **Keine toten Routes**, keine fehlenden.

- **Info** — Tab `pilots` ist UI-mäßig als "Diagnostics" gelabelt (`App.tsx:29`) — der Key ist historisch. Kein Bug, aber Onboarding-stolperfalle.

---

## 8. Webapp Components

Alle Komponenten in `webapp/src/components/` und `webapp/src/tabs/` sind via Imports referenziert (siehe Grep §3). Inklusive `_ApproachStabilityCard`, `_LandingQualityCard`, `_ApproachChart` (via `LandingAnalysis.tsx:9-11`).

- **Info** — `webapp/src/data/airports.ts` `airportCoord` + `webapp/src/data/geocode.ts` (`readAirportCache`, `geocodeNominatim`) sind in LiveMap + Heatmap genutzt. Keine toten.

---

## 9. Sicherheit auf API-Ebene

- **High** — `POST /api/login` (`server.ts:132`) hat kein Rate-Limit + kein Lockout. Bcrypt-12 verlangsamt zwar Brute-Force erheblich (~250 ms/Versuch auf VPS), aber ein Angreifer kann trotzdem 4 Versuche/Sekunde fahren. Empfehlung: `@fastify/rate-limit` mit z.B. 10 Login-Versuche / 15 min / IP.
- **High** — `POST /api/provision` (`server.ts:112`) ebenfalls ohne Rate-Limit + ruft pro Request `fetch()` an `phpvmsBaseUrl/api/user`. Ein Bot kann phpVMS via diesen Proxy DDoS'en oder API-Keys probieren. `validatePhpVmsKey` (`provision.ts:34`) hat zwar `if (apiKey.length < 10) return null` aber das ist keine Brute-Force-Defense.
- **High** — `POST /api/admin/updates/install` (`server.ts:935`) und `POST /api/admin/system/reboot` (`server.ts:980`) führen `sudo apt-get upgrade` bzw. `shutdown -r` aus. `requireAuth` schützt, aber: kein CSRF-Schutz. Cookie ist `sameSite:lax` (`server.ts:140`), das stoppt POST-aus-Cross-Site, **aber** ein XSS in der Webapp (z.B. eingeschmuggelt via PIREP-Notes) würde den Server rebooten. Empfehlung: zusätzlich `Origin`-Header prüfen.
- **High** — `POST /api/admin/jsonl-import` (`server.ts:819`) nimmt einen `file_path` aus dem Body und macht zwar `resolve(file_path).startsWith(resolve(FLIGHT_LOGS_DIR))` — aber FLIGHT_LOGS_DIR ist hier ein **hardcoded** Unix-Pfad `"/var/lib/aeroacars-recorder/flight-logs"` (`server.ts:747`), während der Recorder die Logs unter `cfg.dbPath/../flight-logs` schreibt (`server.ts:56`). Falls die beiden auseinanderlaufen (Dev mit DB_PATH=./data/dev.db), kann der Pfadcheck **nichts** prüfen weil das Verzeichnis nicht existiert — Endpoint gibt 400 zurück, also kein Exploit. **Aber:** Die Hardcode ist fragil. Empfehlung: aus `cfg.dbPath` ableiten.
- **Medium** — `provisioned_pilots.password` wird im Klartext in der SQLite-DB gespeichert (`db.ts:269-280`). Begründet weil Mosquitto die Klartext-Passwort kennen muss um sie wieder zu hashen — aber DB-Backups/-Leaks legen alle Pilot-MQTT-Logins offen. Empfehlung: zumindest Spalte mit Disk-at-Rest-Verschlüsselung (z.B. SQLCipher) oder DB-File-Permissions auf 0600 / 0640 fixiert + dokumentieren.
- **Medium** — Admin-Sessions liegen in-memory (`auth.ts:17`) — bei Recorder-Restart sind alle Admin-Cookies invalid → forced re-login. Akzeptabel für single-tenant, aber: GC-Interval (`auth.ts:50`) ist 1h, max 7d TTL. Kein Limit auf Anzahl gleichzeitiger Sessions/User — moderate Speicher-Leak-Risiko bei Cookie-Reuse-Skripts.
- **Medium** — `req.headers["x-pirep-id"]` aus dem flight-logs-Upload (`server.ts:616`) und `pirep.pirep_id` aus DB (`server.ts:1228`) werden sanitisiert auf `[A-Za-z0-9_-]`. OK so. Aber: Die Cross-Pilot-Authorization in `flight-logs/upload` (`server.ts:626`) prüft `findSessionByPirepForPilot(va, pilot, rawPirepId)`. Achtung: wenn pirep_id global eindeutig ist (UNIQUE constraint an pireps.pirep_id `db.ts:169`), funktioniert das. Falls je zwei Sessions denselben pirep_id-string bekommen, könnte Pilot A das Log von Pilot B überschreiben. Kein aktueller Bug, defensive Anmerkung.
- **Low** — `/api/airports/:icao/metar` (`server.ts:485`) proxied an aviationweather.gov ohne Server-Cache. Bei viel Traffic = unnötige Last + potenzieller Block durch NOAA. Empfehlung: 5-min-In-Memory-Cache (Comment im Code erwähnt das schon).
- **Info** — Authorization-Modell ist "all admins see all VAs" by-design. Multi-Tenant würde Filter auf `req.user.va_prefix` brauchen — aktuell singel-VA (`gsg`).

---

## 10. MQTT-Auth

- **Info** — `recorder/src/config.ts:43-44` liest `MQTT_USERNAME` + `MQTT_PASSWORD` aus Env (systemd `EnvironmentFile=/etc/aeroacars-recorder.env`). Kein Klartext im Repo gefunden.
- **Info** — `vps/mosquitto/passwords.example` enthält nur Demo-Werte (geprüft) → OK.
- **Info** — Mosquitto lauscht nur auf `127.0.0.1:1883` + `127.0.0.1:1884` (`vps/mosquitto/mosquitto.conf` ll. 27-35), exposed via Caddy als `wss://live.kant.ovh/mqtt` mit TLS-Termination. `allow_anonymous false` + `password_file` + `acl_file` aktiv. **Gut konfiguriert.**
- **Medium** — `vps/sudoers.d-aeroacars:11` Regel `sed -i */^user pilot_*/etc/mosquitto/acl.conf` benutzt Wildcards in der Mitte des Cmnd — sudoers-Patterns sind Glob, nicht Regex. Damit kann z.B. `sed -i 's/foo/bar/' /etc/passwd /etc/mosquitto/acl.conf` als pseudo-Match durchgehen. **Bitte überprüfen** ob der pattern tatsächlich strikt ist (sudo's `-n` mode + `parse_error` aus visudo). Sicherer wäre eigenes Helper-Script wie `aeroacars-add-pilot` für Delete.
- **Low** — `pilotMgmt.ts:64-67` führt `sed -i "/^user ${user}$/,/^$/d"` mit user aus dem URL aus, validiert mit `^pilot_[a-zA-Z0-9_-]+$`. Validierung ist OK; trotzdem fragil wenn ACL-Format sich ändert. Auch hier Helper-Script wäre robuster.

---

## 11. Dependencies-Audit

### recorder/

```
@fastify/static     8.3.0 → 9.1.3   (Major, Security-Fix s.u.)
@types/bcrypt       5.0.2 → 6.0.0   (Major)
@types/node         22.19 → 25.7    (Major, ESM)
bcrypt              5.1.1 → 6.0.0   (Major, Node20+)
better-sqlite3      11.10 → 12.9    (Major)
typescript          5.9.3 → 6.0.3   (Major)
zod                 3.25  → 4.4     (Major, Breaking)
```

- **High** — `@fastify/static@8.3.0` hat **2 moderate CVEs** (`npm audit`):
  - GHSA-pr96-94w5-mx2h (Path traversal in directory listing, CVSS 5.3)
  - GHSA-x428-ghpx-8j92 (Route guard bypass via encoded path separators, CVSS 5.9)
  Beide gefixt in `9.x`. **Upgrade-Pfad ist semver-major** — Breaking Changes prüfen, aber Fix wird empfohlen. Aktuell: Recorder served Webapp-Dist nur unter `/admin/` mit `decorateReply: false` + eigenem `setHeaders` (`server.ts:66-101`). Risiko-Bewertung: directory-listing ist nicht enabled → CVE-1 wirkt wahrscheinlich nicht. CVE-2 (route guard bypass) könnte theoretisch andere `app.get`-Routen unter `/admin/*` umgehen — aktuell gibt es keine, also Risk ≈ Medium.

### webapp/

- **Low** — `npm audit` für webapp: **0 vulnerabilities** (43 prod / 444 dev deps). Sauber.
- **Info** — `maplibre-gl 4.7 → 5.24`, `vite 6 → 8`, `typescript 5.9 → 6.0` sind Majors verfügbar, aber kein Sicherheitsdruck.

### Sonstiges

- **Info** — `recorder/package.json:8` Script `dev` nutzt `tsx watch` — gut. Build via `tsc -p tsconfig.json` (kein Bundler). Production-Footprint ist 9 prod-deps — minimal.
- **Info** — `package-lock.json` beider Pakete ist im Repo committed → reproduzierbare Builds. **Gut.**

---

## VPS-Config-Befunde

- **Info** — `vps/deploy-recorder.sh` und `vps/bootstrap.sh` sind **LF-only** (geprüft mit `od -c` byte-level). Kein CRLF-Problem auf aktuellem Branch. (Falls User von einem früheren Branch sprach: derzeit clean.)
- **Low** — `vps/deploy-recorder.sh:17` hardcoded Branch `claude/aeroacars-windows-app-6lPsp`. OK für Single-Branch-Setup, aber: Branch-Name suggeriert "Windows-App"-Feature-Branch und ist nicht semantisch das Main. Empfehlung: `main` mergen oder Branch sauber benennen (`production` / `live`).
- **Low** — `vps/systemd/aeroacars-recorder.service:22` `NoNewPrivileges=false` — bewusst gelockert wegen `sudo aeroacars-add-pilot`. Im Kommentar dokumentiert. Akzeptable Privilege-Trade-Off, aber: alternative wäre `CapabilityBoundingSet` + capabilities statt sudo.

---

## Top-10-Findings (priorisiert)

| # | Severity | Bereich | Was | Wo |
|---|---|---|---|---|
| 1 | **High** | Security/Deps | `@fastify/static@8.3.0` hat 2 moderate CVEs (path traversal + route bypass) — Major-Upgrade auf 9.1.3 erforderlich. | `recorder/package.json:15` |
| 2 | **High** | Security/API | `POST /api/login` ohne Rate-Limit — Brute-Force gegen Admin-Konten möglich (Bcrypt-12 mildert, blockt nicht). | `recorder/src/server.ts:132` |
| 3 | **High** | Security/API | `POST /api/provision` ohne Rate-Limit + proxied jeden Request an phpVMS → DDoS-Hebel + API-Key-Brute-Force. | `recorder/src/server.ts:112` |
| 4 | **High** | Security/API | `POST /api/admin/{updates/install,system/reboot}` ohne CSRF/Origin-Check — XSS würde Server-Reboot triggern. | `recorder/src/server.ts:935, 980` |
| 5 | **Medium** | Doku | `client-mqtt-extension/*.draft` + `docs/aeroacars-integration-spec.md` (v1) sind veraltet — Realität: Publisher läuft live im Pilot-Client. | `client-mqtt-extension/`, `docs/aeroacars-integration-spec.md` |
| 6 | **Medium** | Architektur | `monitor/` (Tauri-App) deprecated im README aber 100% Tauri-Code + Build-Targets im main Branch — sollte archiviert werden. | `monitor/` |
| 7 | **Medium** | Security/VPS | `vps/sudoers.d-aeroacars:11` Regel mit Wildcards in `sed`-Cmnd ist riskant — explizites Helper-Script empfohlen. | `vps/sudoers.d-aeroacars:11` |
| 8 | **Medium** | Security/Data | `provisioned_pilots.password` im Klartext in SQLite. DB-Backup-Leak = alle MQTT-Pilot-Logins kompromittiert. | `recorder/src/db.ts:269-280` |
| 9 | **Medium** | Version | `recorder` + `webapp` package.json zeigen "0.1.0" während Code-Realität bei v0.7.11 ist — kein Deploy-Verify möglich. | `recorder/package.json:3`, `webapp/package.json:4` |
| 10 | **Low** | Dead Code | `db.listTouchdowns()` + `db.listPireps()` ungenutzt (ersetzt durch `*WithAircraft`-Varianten). | `recorder/src/db.ts:1804, 1947` |

---

## Quick-Wins (≤ 15 min Aufwand)

1. `recorder/src/db.ts` — `listTouchdowns` + `listPireps` löschen (Finding #10).
2. `client-mqtt-extension/` löschen oder mit Deprecated-Banner in README (Finding #5).
3. `docs/aeroacars-integration-spec.md` → `docs/archive/` verschieben + Hinweis im v2-Spec (Finding #5).
4. Beide `package.json` auf realistische Version setzen (z.B. `0.7.11` synchron) (Finding #9).
5. `@fastify/static` auf 9.1.3 bumpen + Smoke-Test `/admin/` (Finding #1).

## Mid-Term (½ Tag)

6. `@fastify/rate-limit` einhängen für `/api/login`, `/api/provision`, `/api/admin/jsonl-import` (Findings #2, #3).
7. Origin-Header-Check für alle `POST /api/admin/system/*` und `/api/admin/updates/install` (Finding #4).
8. `monitor/` in eigenen Archiv-Branch verschieben (Finding #6).
