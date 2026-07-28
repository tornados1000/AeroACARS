# 🔍 AeroACARS — Master Audit Report

**Date**: 2026-05-12
**Scope**: Pilot Client (`E:\CloudeAcars`) + aeroacars-live VPS (`E:\aeroacars-live`)
**Versions**: Pilot Client v0.7.12 (live released), Recorder/Webapp HEAD = `f647577`
**Mode**: Read-only QA — no changes made

**Meta-QA by User (post-audit) — corrections below in the "QA Deviations" section:**
- `cargo audit` was NOT runnable locally (`cargo-audit` not installed) → Rust dep CVE statement is best-effort manual only, not tool-verified
- FSUIPC code paths are clean (hard ban respected), but doc mentions are >2 (multiple READMEs/specs), report number was too low
- Local `latest.json` bundle artifact shows `0.5.38` — this is just a stale build-folder leftover; the remotely deployed `latest.json` correctly shows `0.7.12`

Three parallel audit agents (Pilot Client, aeroacars-live, Security) were run. Detailed reports are located at:
- `docs/audit/pilot-client-audit.md`
- `docs/audit/aeroacars-live-audit.md`
- `docs/audit/security-audit.md`

---

## 🚨 EXECUTIVE SUMMARY — Critical + High

| # | Sev | Category | What | Where |
|---|---|---|---|---|
| **C1** | 🔴 **CRITICAL** | Secret Leak | **Discord webhook token committed in public GitHub repo** — anyone can post spam/phishing into the GSG Discord | `client/src-tauri/src/discord.rs:27` |
| **C2** | 🔴 **CRITICAL** | Privilege Escalation | The `aeroacars` user has passwordless sudo `NOPASSWD:ALL`, and webapp endpoints call `sudo apt-get`/`shutdown` — an admin cookie leak equals full root | `aeroacars-live/vps/bootstrap.sh:67` |
| H1 | 🟠 High | Brute Force | No rate limit on `/api/login`, `/api/provision`, `/api/forms/aircraft` — credential stuffing + DoS amplification possible | `recorder/src/server.ts` |
| H2 | 🟠 High | RCE Risk | `apt-get upgrade` + `shutdown` as admin API endpoints — apt postinstall hooks effectively equal arbitrary command execution as root | `recorder/src/server.ts` admin routes |
| H3 | 🟠 High | Dep CVE | `@fastify/static@8.3.0` → 2 moderate CVEs (path traversal + route guard bypass). Fix in 9.1.3 (major bump) | `recorder/package.json` |
| H4 | 🟠 High | Key Hygiene | Tauri updater private key `client/aeroacars-updater.key` sits in the repo working tree (`.gitignore` correct, history clean, but one `git add -f` would be catastrophic) | `client/aeroacars-updater.key` |
| H5 | 🟠 High | Dep CVE | `tar` chain via `bcrypt → @mapbox/node-pre-gyp` in Recorder, 6 CVEs, top CVSS 8.2 (install-time only, but should be fixed) | `recorder/package.json` |
| H6 | 🟠 High | Dead Code | Discord Rich Presence block ~170 LOC dead since v0.4.0 — promise "wiring comes in v0.4.5", we're at v0.7.12 | `client/src-tauri/src/discord.rs:491-659` |
| H7 | 🟠 High | Dead Code | 4 orphaned React components with no importer | `client/src/components/`: `Dashboard.tsx`, `FlightInfoPanel.tsx`, `MassPanel.tsx`, `PhaseTimeline.tsx` |
| H8 | 🟠 High | Dead Code | 6 Tauri commands registered in `generate_handler!` but never called via `invoke<>` anywhere in the frontend | `client/src-tauri/src/lib.rs`: `get_minimize_to_tray`, `get_simbrief_settings`, `landing_get`, `ofp_callsign_warning_dismiss`, `xplane_uninstall_plugin`, `detect_running_sim` |

---

## ⚡ RECOMMENDED ACTION ORDER

### Immediately (tonight / tomorrow morning)

1. **C1 — Rotate the Discord webhook**:
   - `Discord → GSG server → Channel Settings → Integrations → Webhooks → delete existing → create new`
   - **Note:** Once rotated, all previously installed Pilot Clients (v0.4.0 through v0.7.12) will STOP posting into the Discord channel — until they update to a new version with the new URL embedded.
   - **Clean-fix idea** (for v0.7.13): don't hardcode the webhook URL, instead fetch it from the Recorder backend (live.kant.ovh) at Pilot Client startup. That makes rotation server-side going forward.
   - Bonus: remove the old webhook token from git history using `git filter-repo` or BFG (caution: rewriting history can break pull requests + forks) — alternatively, just publicly acknowledge it and leave it, since it's being rotated anyway.

2. **C2 — Enforce the sudoers whitelist**:
   - Change `bootstrap.sh:67`: delete the `NOPASSWD:ALL` line, activate the existing `sudoers.d-aeroacars` (= surgical whitelist) as the sole source of sudo rights.
   - **Before** doing so, test on the VPS whether the admin endpoints (`updates/install`, `system/reboot`) still work — they now need explicitly whitelisted commands.

### This Week

3. **H1 — Introduce rate limiting** on `/api/login` + `/api/provision` (e.g. `@fastify/rate-limit`, 5 attempts/15min/IP).
4. **H2 — Secure admin endpoints**: 2FA confirmation flow or at least a re-auth prompt before `updates/install` + `system/reboot`. Plus CSRF/origin check.
5. **H3 — Major bump `@fastify/static`** to 9.1.3 (read breaking-change notes, then update + redeploy).
6. **H4 — Move the updater key** outside the repo (e.g. `~/.config/aeroacars-keys/updater.key`), GitHub secrets remain unchanged.
7. **H5 — Move the `bcrypt` dep** to a version without `@mapbox/node-pre-gyp` (or migrate to pure-JS `bcryptjs`).

### Next 2 Weeks

8. **H6 + H7 + H8 — Code cruft cleanup**: remove the Discord RP block, delete the 4 orphaned React components, remove the 6 orphan Tauri commands (or complete the frontend wiring if planned).
9. Go through the **M findings (see below)** and decide which to remove.
10. **Fix MEMORY.md** (see cross-cutting section below).

---

## 🔄 CROSS-CUTTING FINDINGS

These points came up independently across multiple audits:

### CC1 — MEMORY.md is wrong about secret storage
User memory says "phpVMS API key is stored via Windows Credential Manager / macOS Keychain".
**Reality:** Since v0.5.15, `client/src-tauri/crates/secrets/src/lib.rs` writes plaintext JSON to `<app_data_dir>/secrets.json` (chmod 0600 on Unix, %APPDATA% ACL on Windows). The module comment explains why (macOS Keychain prompt-loop). The `keyring` dependency is still in `Cargo.toml` for `migrate_from_keyring()` from v0.5.15 — it can be removed after ~30 releases.

→ **Action**: update MEMORY.md + remove the `keyring` dep and migration code (see Pilot Client Audit item 6).

### CC2 — Stale specs & drafts
- Pilot Client: `docs/spec/*.md` with "wiring comes in v0.4.5", "patch in v0.7.7" (= either happened or was abandoned)
- aeroacars-live: `client-mqtt-extension/*.draft` + `docs/aeroacars-integration-spec.md` v1 say "Phase 0, no implementation" — but AeroACARS has been publishing live over MQTT since v0.4

→ **Action**: a one-hour drafts/specs cleanup session: archive under `docs/spec/historical/` or delete.

### CC3 — Version drift across multiple projects
- Pilot Client v0.7.12 ✅ consistent (just fixed)
- Recorder/Webapp `package.json` both still at `0.1.0` while the code is v0.7.11-aligned

→ **Action**: give Recorder/Webapp `package.json` their own aeroacars-live versioning scheme (e.g. `1.0.0` + semver-per-recorder-release-tag), or align them with the Pilot Client.

### CC4 — Deprecated sub-project `monitor/`
README says deprecated, but it lives on fully in the branch (the Tauri desktop variant of the webapp admin tool).

→ **Action**: decide — either archive as "historical/v0.4-monitor/" or delete entirely. User memory says "Monitor app is Windows-only, admin-only" — if the webapp fully replaces it, `monitor/` can go.

---

## ✅ NEGATIVE FINDINGS — what was checked and is clean

So you can see that most of the housekeeping is OK:

**Pilot Client:**
- 0 TypeScript build errors / warnings
- 0 FSUIPC code paths (user hard ban respected) — **7 doc mentions** in `README.md`, `client/README.md`, `client/src-tauri/crates/sim-msfs/Cargo.toml` (description string), `docs/architecture.md`, `docs/decisions/0002-msfs-simconnect-only.md`, `docs/decisions/README.md`, `docs/pmdg-sdk-integration.md` — all as "explicitly not used" markers, no code risk. The original audit number "2" was wrong.
- All versions consistent at 0.7.12 (package.json / tauri.conf.json / Cargo.toml / Cargo.lock)
- i18n DE/EN parity 100% (881/881 keys), IT has 1 extra key
- Volanta / LandingRate-1.lua are algorithm references (documentation), not live endpoints
- `pre-v0.5.x` backward compat (15+ spots) is legitimate for stored legacy JSON files
- Rust deps current, no crate more than 1 minor behind

**aeroacars-live:**
- TS build clean (Recorder + Webapp 0 errors)
- Webapp `npm audit` = 0 vulnerabilities
- Mosquitto broker configured correctly: `allow_anonymous false`, both listeners on 127.0.0.1, per-pilot ACL (`aeroacars/<va>/<pilot>/#`) — Pilot A cannot read/publish Pilot B's topics
- 55 of 61 REST endpoints behind `requireAuth`, 6 public (all legitimately justified, e.g. `/api/ping`)
- All 10 webapp tabs mapped + reachable, no dead routes
- The CRLF issue on deploy-recorder.sh is NOT current (bytes verified LF-only via `od -c`) — was a one-off SSH glitch
- TLS via Caddy, automatic certificate management
- MQTT auth via env variables, not in the repo

**Security specific:**
- No PEM keys, JWTs, hex tokens, DB URLs with credentials in either repo (except C1+H4)
- 0 SQL injection (all queries parameterized; template strings only for whitelisted column names)
- 0 path traversal (flight log upload + JSONL import have sanitization + `resolve()` prefix checks)
- Bcrypt rounds 12, `timingSafeEqual` for Basic Auth
- No CORS misconfiguration (Fastify default = no CORS, webapp same-origin)
- Updater uses HTTPS-only, embedded Minisign pubkey verifies signatures — the model is sound once H4 is addressed
- No secret leaks in `tracing::*` logs (Pilot Client) or the Recorder Fastify logger
- SQLite payloads contain only telemetry, no API keys

---

## 📋 FULL FINDING INDEX (all severities, Pilot Client)

| Sev | # | What | Where |
|---|---|---|---|
| H | 1 | Discord RP block dead since v0.4.0 | `discord.rs:491-659` |
| H | 2 | 4 orphan React components | `src/components/` |
| H | 3 | 6 orphan Tauri commands | `lib.rs` `generate_handler!` |
| M | 4 | `fcu_debounce()` + 8 struct fields (Fenix plan abandoned) | `lib.rs:2198, 16368` |
| M | 5 | Build warnings: `current_premium_status`, `count` | `lib.rs:12241, 7230` |
| M | 6 | `secrets::migrate_from_keyring()` + `keyring` dep (v0.5.15 migration, 30+ releases ago) | `crates/secrets/src/lib.rs` |
| M | 7 | 3 genuinely dead i18n keys: `tabs.dashboard`, `landing.peak_vs`, `landing.plan_tow`, `landing.plan_ldw` | `locales/{de,en,it}/common.json` |
| L | 8 | Discord `EventContext` fields `airline_icao` + `fuel_used_kg` set but not read | `discord.rs:52` |
| L | 9 | Workspace dep `schemars = "0.8"` declared but unused | `Cargo.toml` |
| L | 10 | Stale promise comments (6× "wiring comes in v0.4.5", 1× "patch in v0.7.7") | various |

## 📋 FULL FINDING INDEX (aeroacars-live)

| Sev | # | What | Where |
|---|---|---|---|
| H | 11 | `@fastify/static` 8.3.0 → 2 CVEs | `recorder/package.json` |
| H | 12 | No rate limit on auth endpoints | `recorder/src/server.ts` |
| H | 13 | Admin endpoints call `sudo` without re-auth | `recorder/src/server.ts` |
| M | 14 | Specs `client-mqtt-extension/*.draft` + `aeroacars-integration-spec.md` v1 stale | `docs/`, `client-mqtt-extension/` |
| M | 15 | `monitor/` Tauri desktop marked deprecated in README, still lives in the branch | `monitor/` |
| M | 16 | `vps/sudoers.d-aeroacars:11` wildcard in `sed` command is glob-fragile | `vps/sudoers.d-aeroacars` |
| M | 17 | `provisioned_pilots.password` plaintext in SQLite | `recorder/src/db.ts` |
| M | 18 | Version drift `package.json 0.1.0` vs code v0.7.11 | `recorder/package.json`, `webapp/package.json` |
| M | 19 | 2 API endpoints with no frontend caller: `/api/provisioned`, `/api/provisioned/:va/:pilot/revoke` | `recorder/src/server.ts` |

## 📋 FULL FINDING INDEX (Security)

| Sev | # | What | Where |
|---|---|---|---|
| **C** | **1** | **Discord webhook token in public repo** | `client/src-tauri/src/discord.rs:27` |
| **C** | **2** | **`NOPASSWD:ALL` + admin RCE endpoints** | `aeroacars-live/vps/bootstrap.sh:67` |
| H | 3 | No rate limit on auth endpoints | see #12 above |
| H | 4 | Updater private key in repo working tree (`.gitignore` correct, history clean) | `client/aeroacars-updater.key` |
| H | 5 | `apt-get upgrade` + `shutdown` as admin endpoints = effectively RCE | see #13 above |
| M | 6 | No security headers in Caddy (CSP, X-Frame-Options, X-Content-Type-Options, Referrer-Policy, HSTS explicit) | `aeroacars-live/vps/caddy/Caddyfile` |
| M | 7 | Tauri webview has `csp: null` — no XSS defense-in-depth | `client/src-tauri/tauri.conf.json` |
| M | 8 | Pilot MQTT passwords plaintext in `provisioned_pilots.password` (design constraint, but document or encrypt at rest) | see #17 above |
| M | 9 | **CC1** — MEMORY.md says "Keychain", reality is plaintext JSON in app data | `crates/secrets/src/lib.rs` |
| M | 10 | `bcrypt → @mapbox/node-pre-gyp → tar` 6 CVEs (install-time only) | `recorder/package.json` |

---

## 🛠 RECOMMENDED FIX PRIORITY PATH

```
IMMEDIATE (Cat: critical, exploitable)
├── C1: Rotate Discord webhook               (5 min in the Discord UI)
└── C2: NOPASSWD:ALL → sudoers whitelist      (1 SSH session, ~30 min with testing)

THIS WEEK (Cat: high, exploitable with some effort)
├── H1+H2: Rate limit + re-auth on admin endpoints   (~3h code+test)
├── H3: @fastify/static 9.x major bump                (~1h + test)
├── H4: Move updater key out of the repo              (~30 min)
└── H5: bcrypt → bcryptjs                              (~1h)

NEXT 2 WEEKS (Cat: medium, cleanup + hygiene)
├── H6+H7+H8: Dead-code cleanup (Discord RP, 4 React, 6 Tauri commands)  (~3h)
├── M1-M3: Archive specs/drafts/monitor/                                  (~2h)
├── M-Various: dead i18n keys, Cargo schemars dep, stale promise comments (~2h)
├── CC1: Fix MEMORY.md + remove keyring migration                        (~1h)
├── CC3: Recorder/Webapp versioning scheme                                (~1h)
├── #6: Caddy security headers (minimal CSP, X-Frame, explicit HSTS)      (~1h)
└── #7: Set Tauri csp instead of null                                       (~30 min)

OPTIONAL (Cat: low + documentation)
├── #9: Remove schemars workspace dep
├── M-Various Pilot Client: clean up stale comments
└── M8: Encrypt provisioned_pilots.password at rest + document "design constraint"
```

**Total effort estimate:** ~20-25h for complete cleanup + security hardening. Critical items alone: ~35 min.

---

## 🔬 QA Deviations / Caveats (identified by the user)

The user cross-checked the audit result themselves. The following points are corrections / clarifications:

| # | Caveat | Consequence |
|---|---|---|
| Q1 | **`cargo audit` was not run locally** — `cargo-audit` is not installed on the build machine. Agent C formulated Rust dep CVE statements best-effort manually, but not tool-verified | **Status of Pilot Client Rust deps = unknown.** Recommendation: run `cargo install cargo-audit && cargo audit` once before the v0.7.13 release |
| Q2 | **FSUIPC doc count corrected** — audit said 2, real number is 7 unique files (see Negative Findings section above) | No functional risk (code paths remain clean), only report accuracy |
| Q3 | **Stale local `latest.json`** in `client/src-tauri/target/release/bundle/nsis/latest.json` shows `0.5.38` | Just a local build artifact from an old build session. The remote `latest.json` at github.com/.../releases/latest/download/latest.json correctly shows `0.7.12`. Can be ignored locally, or delete `target/release/bundle/` locally |
| Q4 | **Audit documents are untracked in git** (`E:\CloudeAcars\docs\audit\` new) | If the report should be kept in the repo long-term → `git add docs/audit/ && git commit`. If not → add a `.gitignore` entry |
| Q5 | **`E:\aeroacars-live` working tree not clean** — `webapp/src/components/LandingAnalysis.tsx` has a local change vs HEAD | Deliberate user change (linter or manual), not to be rolled back. Can be folded into the next commit |

### Completed build gates (verified by the user)
- Client: `npm run build` ✅, `npm test` ✅ (39/39), `cargo check` ✅ (4 warnings)
- Webapp: `npm run build` ✅
- Recorder: `npm run build` ✅
- npm audit: Client + Webapp = 0 vulns; Recorder = 3 vulns (see H3+H5)
- Public REST endpoints (6) confirmed: `healthz`, `provision`, `login`, `flight-logs/upload`, `vapid-public-key`, `forms/aircraft`
- i18n parity: DE 881, EN 881, IT 882 (1 extra key) — confirmed

---

## 📁 Reference Reports

- `pilot-client-audit.md` — full list with `path:line` quotes (Agent A, 16 KB)
- `aeroacars-live-audit.md` — endpoint table + SQL schema drift (Agent B, 18 KB)
- `security-audit.md` — per-finding severity + impact + recommendation + appendix (Agent C, 31 KB)

If you have questions about individual findings, open the corresponding detail report — it contains the exact context, affected file areas, and suggested fix strategy for each.
