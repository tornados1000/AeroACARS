# aeroacars-live — Read-Only QA Audit

**Audit Date:** 2026-05-12
**Branch:** `claude/aeroacars-windows-app-6lPsp` (HEAD `f647577`)
**Scope:** recorder (Node/Fastify), webapp (React/Vite), vps/, docs/, client-mqtt-extension/
**Method:** static + `tsc --noEmit` + `npm outdated/audit` (read-only)

Severity: **Critical** / **High** / **Medium** / **Low** / **Info**.
`repo:path:line` is relative to `E:/aeroacars-live/`.

---

## 1. Dead / Unused TypeScript Code

`tsc --noEmit` ran for recorder and webapp with no output (= 0 errors, 0 warnings). No unused imports in the strict sense.

- **Low** — `recorder/src/db.ts:1804` `listTouchdowns(limit)` is called nowhere (replaced by `listTouchdownsWithAircraft` at server.ts:202). Recommendation: delete or mark `@deprecated`.
- **Low** — `recorder/src/db.ts:1947` `listPireps(limit)` similarly unused (replaced by `listPirepsWithAircraft` at server.ts:315). Recommendation: delete.
- **Info** — `recorder/src/db.ts:30` `interface AdminUser` is exported but only used via the `getAdmin()` return value — could remain internal. No action item.
- **Info** — No dead code in the webapp/recorder per TypeScript strict mode. `recoderRow` helpers at server.ts:1383/1393 are both linked.

---

## 2. Version Drift

- **Medium** — Both package.json files claim `"version": "0.1.0"` (`recorder/package.json:3`, `webapp/package.json:4`), while the repo + Pilot Client are at v0.7.11. CHANGELOG / release tags live only in Git. Recommendation: either keep semver in sync with the Pilot Client, or expose a `RECORDER_VERSION` string from `index.ts` at `/api/healthz` so deploys are verifiable.
- **Info** — Webapp code references v0.5.18, v0.5.23, v0.5.25, v0.5.26, v0.5.34, v0.5.49, v0.7.6, v0.7.7, v0.7.11 in comments. Most describe historical schema versioning and make sense. No contradictory v0.5.x claims found.
- **Low** — `recorder/src/server.ts:60,74` comment "v0.5.x: Diagnostics tab needs …" and the `cacheControl` block are no longer "in flux" but stable — marker is stale, harmless.

---

## 3. Deprecated / Stale Features

- **Medium** — `monitor/` (Tauri desktop app) is officially deprecated (README.md:36, monitor/README.md:1) — but remains in the repo and is not covered by the `tsc` scope. Since the memory constraint states the Monitor is Windows-only/admin-only, the Tauri app should **definitely** be archived or at least set to `.gitattributes export-ignore`. Recommendation: move to its own branch and remove from main.
- **Low** — `webapp/src/data/phaseColors.ts:23` phase `ON_BLOCK` is explicitly a "legacy alias — pre-v0.5.18 clients". Keep as long as legacy data exists in the DB, but add a tracking comment for when it's safe to remove.
- **Low** — `_ApproachStabilityCard.tsx:18` `hasV2` check tolerates legacy pre-v0.5.25 touchdowns. Identical logic in `_LandingQualityCard.tsx`. Fine as-is, no cleanup needed.
- **Info** — The underscore prefix (`_ApproachStabilityCard`, `_ApproachChart`, `_LandingQualityCard`) is **not** a deprecation marker — these files are imported by `LandingAnalysis.tsx` (see grep). Convention appears to be: "sub-component, used only by one parent component". Recommendation: document in CONTRIBUTING / README, or move the files into `components/landing/`.

---

## 4. Stale Specs / Draft Files

- **Medium** — `docs/aeroacars-integration-spec.md` (v1 manual token flow) has been superseded by `docs/aeroacars-integration-spec-v2.md` (auto-provisioning) — the v1 spec references the "Live Monitor Desktop App" as its target audience, which is no longer accurate (Monitor deprecated). Recommendation: move v1 to `docs/archive/` or add a deprecated banner.
- **Medium** — `client-mqtt-extension/{Cargo.toml.draft, lib.rs.draft, README.md}` explicitly reference Phase 0 / "no implementation". Reality: the AeroACARS Pilot Client has been publishing live over MQTT since v0.4+ (see commits in the main repo). Recommendation: delete the entire `client-mqtt-extension/` or replace it with a reference to the main repo's crate.
- **Low** — `docs/topic-schema.md` is current (references `block`/`takeoff` channels from v0.5.14+) — keep.
- **Info** — `docs/architecture.md`, `docs/auth-model.md` have no date / version. Recommendation: add `last-updated: YYYY-MM-DD` frontmatter.

---

## 5. API Endpoints Audit

**Endpoints in `recorder/src/server.ts` (61 routes + WS `/api/live`):** All except 6 have `preHandler: requireAuth`.

### 5a. Without `requireAuth` (justified or problematic)

| Endpoint | Justification | Severity |
|---|---|---|
| `GET /api/healthz` (`server.ts:108`) | public health probe | OK |
| `POST /api/provision` (`server.ts:112`) | auto-provisioning, validates the phpVMS API key in `provision.ts` | Medium — see §9 |
| `POST /api/login` (`server.ts:132`) | login, bcrypt | Medium — no rate limit (§9) |
| `POST /api/flight-logs/upload` (`server.ts:578`) | HTTP Basic against `provisioned_pilots` + timing-safe compare | OK |
| `GET /api/admin/push/vapid-public-key` (`server.ts:1012`) | public key, harmless | OK |
| `POST /api/forms/aircraft` (`server.ts:1122`) | validates the `X-Forms-Token` shared secret from the DB | OK |

### 5b. Endpoints WITHOUT a webapp caller (server has them, webapp doesn't use them)

- **Info** — `GET /api/provisioned` and `POST /api/provisioned/:va/:pilot/revoke` (`server.ts:121, 123`) — no callers in `webapp/src/`. There is a `db.listProvisionedPilots()` backend path, but no UI element. Recommendation: build a webapp admin view OR remove the endpoints.
- **Info** — `GET /api/touchdowns/forensik` (`server.ts:707`) → `api.touchdownForensik` at `api.ts:230`, called in `Touchdowns.tsx:272`. **OK.**
- **Info** — All remaining endpoints are called via `webapp/src/api.ts` or directly via `fetch(...)` in the tabs (`/api/admin/jsonl-files`, `/api/admin/jsonl-import`, `/api/pireps/:id/jsonl`, `/api/pireps/:id/export`, `/api/flights/:va/:pilot/export`, `/api/admin/push/*` — all found again).

### 5c. Auth model note

- **Info** — `requireAuth` (`server.ts:1373`) authenticates **ONLY** the admin cookie; it does **not** factor the VA/pilot params in the URLs into the filter. This is intentional (admin-only tool), but worth documenting: every admin sees every pilot of every VA. See §9.

---

## 6. SQL Schema vs Code Drift

- **Low** — `positions` columns `simulation_rate`, `paused`, `autopilot_master`, `light_beacon`, `light_strobe`, `light_landing` (`db.ts:174-203`) are written by CLI tools + the importer but **never read** in the webapp (grep: 0 hits in `webapp/src`). They're telemetry/forensics-only — OK, but at roughly ~9 bytes per column per position × ~30d retention that adds up. Recommendation: at least document whether this is a deliberate archival choice.
- **Info** — `flight_session_stats` (`db.ts:207`) has 23 columns, all read in `recomputeSessionStats()` + telemetry endpoints. Nothing dead.
- **Low** — No index on `positions(ts)` alone — all queries go through `(va_prefix, pilot_id, ts)` (covered by `idx_positions_pilot_ts`). For cross-pilot heatmap queries (e.g. `allTouchdownsForHeatmap`) this doesn't help, but those run through the `touchdowns.ts` index instead. **OK.**
- **Low** — `pireps.payload_json` is filtered via full-text search in `searchPireps` (server.ts:330) (the `q.q` param) — but there's no FTS index. Harmless below ~2000 PIREPs/year, noticeable above 10k.
- **Info** — `touchdowns` has had `UNIQUE(va_prefix, pilot_id, ts)` since v0.5.34 (`db.ts:557`) — robust against QoS-1 re-delivery. The migration is written defensively.

---

## 7. Webapp Routes

Tab keys from `useHashRoute.ts:21`: `live, flights, landings, reports, pilots, history, trends, heatmap, system, admin`. All 10 are mapped to a component in `App.tsx:118-130`. **No dead routes**, none missing.

- **Info** — Tab `pilots` is labeled "Diagnostics" in the UI (`App.tsx:29`) — the key is historical. Not a bug, but an onboarding trip-hazard.

---

## 8. Webapp Components

All components in `webapp/src/components/` and `webapp/src/tabs/` are referenced via imports (see grep §3). Including `_ApproachStabilityCard`, `_LandingQualityCard`, `_ApproachChart` (via `LandingAnalysis.tsx:9-11`).

- **Info** — `webapp/src/data/airports.ts` `airportCoord` + `webapp/src/data/geocode.ts` (`readAirportCache`, `geocodeNominatim`) are used in LiveMap + Heatmap. Nothing dead.

---

## 9. Security at the API Level

- **High** — `POST /api/login` (`server.ts:132`) has no rate limit and no lockout. Bcrypt-12 does slow brute force considerably (~250 ms/attempt on the VPS), but an attacker can still run 4 attempts/second. Recommendation: `@fastify/rate-limit` with e.g. 10 login attempts / 15 min / IP.
- **High** — `POST /api/provision` (`server.ts:112`) also has no rate limit and makes a `fetch()` call to `phpvmsBaseUrl/api/user` on every request. A bot could DDoS phpVMS through this proxy or probe API keys through it. `validatePhpVmsKey` (`provision.ts:34`) does have `if (apiKey.length < 10) return null`, but that's not a brute-force defense.
- **High** — `POST /api/admin/updates/install` (`server.ts:935`) and `POST /api/admin/system/reboot` (`server.ts:980`) execute `sudo apt-get upgrade` and `shutdown -r` respectively. `requireAuth` protects them, but: no CSRF protection. The cookie is `sameSite:lax` (`server.ts:140`), which stops cross-site POSTs, **but** an XSS in the webapp (e.g. smuggled in via PIREP notes) would be able to reboot the server. Recommendation: additionally check the `Origin` header.
- **High** — `POST /api/admin/jsonl-import` (`server.ts:819`) takes a `file_path` from the body and does check `resolve(file_path).startsWith(resolve(FLIGHT_LOGS_DIR))` — but FLIGHT_LOGS_DIR here is a **hardcoded** Unix path `"/var/lib/aeroacars-recorder/flight-logs"` (`server.ts:747`), while the recorder actually writes logs under `cfg.dbPath/../flight-logs` (`server.ts:56`). If the two diverge (e.g. dev with DB_PATH=./data/dev.db), the path check can't validate **anything** because the directory doesn't exist — the endpoint returns 400, so no exploit results. **But:** the hardcode is fragile. Recommendation: derive it from `cfg.dbPath`.
- **Medium** — `provisioned_pilots.password` is stored in plaintext in the SQLite DB (`db.ts:269-280`). Justified because Mosquitto needs the plaintext password to re-hash it — but DB backups/leaks expose every pilot's MQTT login. Recommendation: at minimum encrypt the column at rest (e.g. SQLCipher) or lock down the DB file permissions to 0600/0640 and document it.
- **Medium** — Admin sessions live in-memory (`auth.ts:17`) — on a recorder restart, all admin cookies become invalid → forced re-login. Acceptable for single-tenant, but: the GC interval (`auth.ts:50`) is 1h, max 7d TTL. No limit on the number of concurrent sessions per user — moderate memory-leak risk from cookie-reuse scripts.
- **Medium** — `req.headers["x-pirep-id"]` from the flight-logs upload (`server.ts:616`) and `pirep.pirep_id` from the DB (`server.ts:1228`) are sanitized to `[A-Za-z0-9_-]`. Fine as-is. But: the cross-pilot authorization check in `flight-logs/upload` (`server.ts:626`) uses `findSessionByPirepForPilot(va, pilot, rawPirepId)`. Note: this works as long as pirep_id is globally unique (UNIQUE constraint on pireps.pirep_id `db.ts:169`). If two sessions ever get the same pirep_id string, Pilot A could overwrite Pilot B's log. Not a current bug, just a defensive note.
- **Low** — `/api/airports/:icao/metar` (`server.ts:485`) proxies to aviationweather.gov with no server-side cache. Under heavy traffic this means unnecessary load and potential blocking by NOAA. Recommendation: 5-minute in-memory cache (already noted in a code comment).
- **Info** — The authorization model is "all admins see all VAs" by design. Multi-tenancy would require filtering on `req.user.va_prefix` — currently single-VA (`gsg`).

---

## 10. MQTT Auth

- **Info** — `recorder/src/config.ts:43-44` reads `MQTT_USERNAME` + `MQTT_PASSWORD` from env (systemd `EnvironmentFile=/etc/aeroacars-recorder.env`). No plaintext found in the repo.
- **Info** — `vps/mosquitto/passwords.example` contains only demo values (checked) → OK.
- **Info** — Mosquitto listens only on `127.0.0.1:1883` + `127.0.0.1:1884` (`vps/mosquitto/mosquitto.conf` ll. 27-35), exposed via Caddy as `wss://live.kant.ovh/mqtt` with TLS termination. `allow_anonymous false` + `password_file` + `acl_file` active. **Well configured.**
- **Medium** — `vps/sudoers.d-aeroacars:11` rule `sed -i */^user pilot_*/etc/mosquitto/acl.conf` uses wildcards in the middle of the command — sudoers patterns are glob, not regex. This means e.g. `sed -i 's/foo/bar/' /etc/passwd /etc/mosquitto/acl.conf` could pass as a pseudo-match. **Please verify** whether the pattern is actually strict (sudo's `-n` mode + `parse_error` from visudo). A dedicated helper script like `aeroacars-add-pilot` for deletion would be safer.
- **Low** — `pilotMgmt.ts:64-67` runs `sed -i "/^user ${user}$/,/^$/d"` with `user` taken from the URL, validated against `^pilot_[a-zA-Z0-9_-]+$`. Validation is OK; still fragile if the ACL format ever changes. A helper script would be more robust here too.

---

## 11. Dependencies Audit

### recorder/

```
@fastify/static     8.3.0 → 9.1.3   (Major, security fix, see below)
@types/bcrypt       5.0.2 → 6.0.0   (Major)
@types/node         22.19 → 25.7    (Major, ESM)
bcrypt              5.1.1 → 6.0.0   (Major, Node20+)
better-sqlite3      11.10 → 12.9    (Major)
typescript          5.9.3 → 6.0.3   (Major)
zod                 3.25  → 4.4     (Major, Breaking)
```

- **High** — `@fastify/static@8.3.0` has **2 moderate CVEs** (`npm audit`):
  - GHSA-pr96-94w5-mx2h (path traversal in directory listing, CVSS 5.3)
  - GHSA-x428-ghpx-8j92 (route guard bypass via encoded path separators, CVSS 5.9)
  Both fixed in `9.x`. **Upgrade path is semver-major** — check breaking changes, but the fix is recommended. Currently: the recorder serves the webapp dist only under `/admin/` with `decorateReply: false` + its own `setHeaders` (`server.ts:66-101`). Risk assessment: directory listing is not enabled → CVE-1 likely doesn't apply. CVE-2 (route guard bypass) could theoretically bypass other `app.get` routes under `/admin/*` — currently there are none, so risk ≈ Medium.

### webapp/

- **Low** — `npm audit` for webapp: **0 vulnerabilities** (43 prod / 444 dev deps). Clean.
- **Info** — `maplibre-gl 4.7 → 5.24`, `vite 6 → 8`, `typescript 5.9 → 6.0` majors are available, but no security pressure.

### Other

- **Info** — `recorder/package.json:8` script `dev` uses `tsx watch` — good. Build via `tsc -p tsconfig.json` (no bundler). Production footprint is 9 prod deps — minimal.
- **Info** — `package-lock.json` for both packages is committed to the repo → reproducible builds. **Good.**

---

## VPS Config Findings

- **Info** — `vps/deploy-recorder.sh` and `vps/bootstrap.sh` are **LF-only** (checked byte-level with `od -c`). No CRLF issue on the current branch. (If the user was referring to an earlier branch: currently clean.)
- **Low** — `vps/deploy-recorder.sh:17` hardcodes the branch `claude/aeroacars-windows-app-6lPsp`. Fine for a single-branch setup, but: the branch name suggests a "Windows app" feature branch and isn't semantically main. Recommendation: merge into `main` or rename the branch properly (`production` / `live`).
- **Low** — `vps/systemd/aeroacars-recorder.service:22` `NoNewPrivileges=false` — deliberately relaxed because of `sudo aeroacars-add-pilot`. Documented in a comment. Acceptable privilege trade-off, but: an alternative would be `CapabilityBoundingSet` + capabilities instead of sudo.

---

## Top 10 Findings (prioritized)

| # | Severity | Area | What | Where |
|---|---|---|---|---|
| 1 | **High** | Security/Deps | `@fastify/static@8.3.0` has 2 moderate CVEs (path traversal + route bypass) — major upgrade to 9.1.3 required. | `recorder/package.json:15` |
| 2 | **High** | Security/API | `POST /api/login` without rate limit — brute force against admin accounts possible (bcrypt-12 mitigates, doesn't block). | `recorder/src/server.ts:132` |
| 3 | **High** | Security/API | `POST /api/provision` without rate limit + proxies every request to phpVMS → DDoS lever + API key brute force. | `recorder/src/server.ts:112` |
| 4 | **High** | Security/API | `POST /api/admin/{updates/install,system/reboot}` without CSRF/origin check — an XSS would trigger a server reboot. | `recorder/src/server.ts:935, 980` |
| 5 | **Medium** | Docs | `client-mqtt-extension/*.draft` + `docs/aeroacars-integration-spec.md` (v1) are stale — reality: the publisher runs live in the Pilot Client. | `client-mqtt-extension/`, `docs/aeroacars-integration-spec.md` |
| 6 | **Medium** | Architecture | `monitor/` (Tauri app) deprecated in the README but 100% Tauri code + build targets remain in the main branch — should be archived. | `monitor/` |
| 7 | **Medium** | Security/VPS | `vps/sudoers.d-aeroacars:11` rule with wildcards in the `sed` command is risky — an explicit helper script is recommended. | `vps/sudoers.d-aeroacars:11` |
| 8 | **Medium** | Security/Data | `provisioned_pilots.password` in plaintext in SQLite. A DB backup leak would compromise every MQTT pilot login. | `recorder/src/db.ts:269-280` |
| 9 | **Medium** | Version | `recorder` + `webapp` package.json show "0.1.0" while the code reality is v0.7.11 — no deploy verification possible. | `recorder/package.json:3`, `webapp/package.json:4` |
| 10 | **Low** | Dead Code | `db.listTouchdowns()` + `db.listPireps()` unused (replaced by `*WithAircraft` variants). | `recorder/src/db.ts:1804, 1947` |

---

## Quick Wins (≤ 15 min effort)

1. `recorder/src/db.ts` — delete `listTouchdowns` + `listPireps` (Finding #10).
2. Delete `client-mqtt-extension/` or add a deprecated banner in the README (Finding #5).
3. Move `docs/aeroacars-integration-spec.md` → `docs/archive/` + add a note in the v2 spec (Finding #5).
4. Set both `package.json` files to a realistic version (e.g. `0.7.11` in sync) (Finding #9).
5. Bump `@fastify/static` to 9.1.3 + smoke-test `/admin/` (Finding #1).

## Mid-Term (½ day)

6. Wire in `@fastify/rate-limit` for `/api/login`, `/api/provision`, `/api/admin/jsonl-import` (Findings #2, #3).
7. Add an origin-header check for all `POST /api/admin/system/*` and `/api/admin/updates/install` (Finding #4).
8. Move `monitor/` into its own archive branch (Finding #6).
