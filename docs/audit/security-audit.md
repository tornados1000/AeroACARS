# Security Audit — AeroACARS (CloudeAcars) + aeroacars-live

**Date:** 2026-05-12
**Scope:** Read-only audit of two repos:
- `E:\CloudeAcars` (public on GitHub as `MANFahrer-GF/AeroACARS`)
- `E:\aeroacars-live` (public on GitHub as `MANFahrer-GF/aeroacars-live`)

**Methodology:** Static review. No runtime testing, no fuzz, no exploitation.
Tools: `npm audit` (read-only). `cargo audit` not available in env, so Rust
deps were spot-checked manually.

---

## 0. Critical / High Summary (read this first)

| # | Severity  | Finding | Location |
|---|-----------|---------|----------|
| C1 | **Critical** | Discord webhook hardcoded in source on a public repo — anyone can post to GSG channel | `CloudeAcars/client/src-tauri/src/discord.rs:27` |
| C2 | **Critical** | App-User `aeroacars` gets `NOPASSWD: ALL` global sudo on VPS — any webapp RCE = full root | `aeroacars-live/vps/bootstrap.sh:67` |
| H1 | **High** | `/api/login` and `/api/provision` have no rate limit → password brute-force + key validation amplification | `aeroacars-live/recorder/src/server.ts:112,132` |
| H2 | **High** | `apt-get upgrade` and `shutdown -r` exposed as authenticated admin endpoints — single stolen cookie = arbitrary system command via package install hooks | `aeroacars-live/recorder/src/server.ts:935,980` |
| H3 | **High** | `tar` transitive dep in recorder (via `bcrypt → @mapbox/node-pre-gyp`) — 6 known CVEs, hardlink path traversal (CVSS 8.2) | `aeroacars-live/recorder/package-lock.json` |
| H4 | **High** | Updater minisign **private** key sits in repo working tree — not committed today, but trivial to leak via PR review, copy-paste, or future `git add .` mishap | `CloudeAcars/client/aeroacars-updater.key` |

Everything below medium is in the per-section list.

---

## 1. Secrets in Repository

### C1 — Hardcoded GSG Discord webhook URL in public repo
- **Severity:** Critical
- **Location:** `CloudeAcars/client/src-tauri/src/discord.rs:27`
- **What:**
  ```
  const WEBHOOK_URL: &str = "https://discord.com/api/webhooks/1501257121235468418/0sLGmj9-LY4sPfsL0iw7s0TQRmI9qgyYcTek147kR0igU1__IXjx8hXafAl-fPmdOp7Q";
  ```
  Committed in `481abb2` (v0.4.0, 2026-04-xx). Module docstring even says
  *"NIE auf einem öffentlichen Repo committen"* — but the repo is public.
- **Impact:** Any GitHub visitor (and the binary itself, where the string is
  trivially `strings`-greppable) can post arbitrary embeds/messages into
  GSG's Discord channel: spam, phishing-looking PIREP-fake-posts,
  staff-impersonation, leak channel ID to scrapers.
- **Empfehlung:**
  1. **SOFORT** rotate webhook in Discord (Server-Settings → Integrations →
     delete webhook, create new).
  2. Decide policy: either move webhook config to phpVMS-fetched config or
     to the live.kant.ovh `/api/discord` settings (already exists for the
     VA side) and let the client pull it after auth. Forks then set their
     own.
  3. The old URL stays in git history forever — assume permanently
     compromised even after rotation. Same applies to `RICH_PRESENCE_APP_ID`
     (`1340818636616634489`, line 34) — less severe (app-id is semi-public
     by design) but document explicitly.

### Info — Webhook on raw.githubusercontent.com for embed images
- **Severity:** Info
- **Location:** `CloudeAcars/client/src-tauri/src/discord.rs:139-154`
- The image URLs (`takeoff.png`, `landing.png`, …) are public asset paths.
  Fine — they're meant to be public.

### Geprüft: PEM markers — keine Findings.
`grep -r '-----BEGIN'` über beide Repos: nichts.

### Geprüft: JWT / hex tokens — keine Findings.
Grep nach 40+ char hex blocks und Bearer-Patterns: nichts in
versioned files (außer der Webhook-URL oben).

### Geprüft: DB-URLs mit eingebetteten Credentials — keine Findings.
SQLite paths nur (`./data/aeroacars-live.db`). Kein `postgres://user:pw@…`.

### Info — Updater public key embedded in `tauri.conf.json`
- **Location:** `CloudeAcars/client/src-tauri/tauri.conf.json:52`
- Public key (base64 of minisign pubkey ID `4D3A4751C0F98408`). That's
  correct — public key MUST be embedded for the updater to verify
  signatures. No action.

### H4 — Updater **private** signing key in working tree
- **Severity:** High (operational, not currently leaked)
- **Location:**
  - `CloudeAcars/client/aeroacars-updater.key` (encrypted minisign secret)
  - `CloudeAcars/client/aeroacars-updater.key.pub`
- **Status:** Both files are `.gitignored` (`.gitignore:131-132`) and
  `git log --all -- client/aeroacars-updater.key*` returns empty — **never
  committed**. The encrypted key requires a passphrase to use; brute force
  on a strong passphrase is impractical.
- **Was passiert wenn ausgenutzt:** If the private key + passphrase ever
  leak, attacker can sign **arbitrary updates that the auto-updater will
  install with the user's privileges**. Since AeroACARS runs in the user
  account and connects to phpVMS, MSFS, etc., a malicious update can do
  whatever the pilot can do (keylog, exfil credentials, drop ransomware).
  This is the highest-impact secret in the project.
- **Empfehlung:**
  1. **Move the file out of the client directory** to a non-repo location
     (e.g. `~/.aeroacars/signing-keys/`). The `.gitignore` rule is a
     safety net, but having the file inside the repo tree means every
     `git status` reminder, every IDE indexer, every backup tool, every
     screenshot of the file explorer sees it. One careless `git add -f`
     leaks it forever.
  2. Verify the passphrase is high-entropy (≥ 20 chars random) and
     stored only in the developer's password manager.
  3. Document the rotation procedure: how to bump pubkey in
     `tauri.conf.json` and roll out a new signed update if the key ever
     leaks. Without that doc, a leak = end of the project.

---

## 2. Hardcoded URLs / Hostnames

### Info — `german-sky-group.eu` hard-locked in pilot client
- **Severity:** Info (deliberate)
- **Location:** `CloudeAcars/client/src-tauri/src/lib.rs:3927`
  ```
  const ALLOWED_PHPVMS_HOST: &str = "german-sky-group.eu";
  ```
- The login command rewrites whatever URL the user types to this host
  (lines 3946-3948, 4182). Code comment is explicit: pre-1.0 only-GSG
  build.
- **Was passiert:** Forks need a source patch to point at their own
  phpVMS. Acceptable for closed beta; **becomes a usability issue** once
  the repo is presented as a "general phpVMS ACARS client" on GitHub
  (which the public-repo README and `tauri.conf.json:45` already do).
- **Empfehlung:** Pre-1.0 OK. For 1.0: move to a build-time env var or
  per-install config, fall back to GSG only if unset. Track in a public
  issue so forks know it's coming.

### Info — `live.kant.ovh` default broker URL
- **Location:** `CloudeAcars/client/src-tauri/src/lib.rs:10139` and
  `aeroacars-live/recorder/src/config.ts:53`
- Default for `PROVISION_BROKER_URL` env. Pilot client receives this from
  `/api/provision` response (`provisionPublicBrokerUrl`), so it's
  configurable server-side — not hardcoded in the binary's hot path. OK.

### Info — GitHub asset URLs (XP-Plugin Download, Discord image assets)
- All `https://github.com/MANFahrer-GF/AeroACARS/...` and
  `https://raw.githubusercontent.com/MANFahrer-GF/AeroACARS/main/...`.
  Acceptable — they're public release assets. Fork-time concern only.

---

## 3. Auth-Bypass-Möglichkeiten

### Auth mechanism (Webapp)
- **Bcrypt + signed cookie session** (`auth.ts:1-55`). Cookie name
  `aal_session`, 7-day TTL, in-memory `Map<sid, Session>`.
- Cookie attrs (`server.ts:138-145`): `httpOnly`, `sameSite=lax`,
  `signed=true`, `secure=cfg.cookieSecure` (env-driven, defaults `true`).
- Session secret via `SESSION_SECRET` env var, enforced by `need()` in
  `config.ts:46`. Good.

### Auth mechanism (Flight-Log-Upload, Pilot→VPS)
- **HTTP Basic-Auth with timingSafeEqual** (`server.ts:578-613`). Uses
  `SHA-256(provisioned_pilots.password)` on both sides to normalize
  length and then `crypto.timingSafeEqual`. Constant-time comparison.
  Solid implementation.

### Endpoint inventory (`requireAuth` = admin-cookie required)
**Public (no auth):**
- `GET /api/healthz` — only `{ok, ts}`. Fine.
- `GET /healthz` (Caddy) — `"ok"`. Fine.
- `POST /api/provision` — body: `{api_key: string}`. Validates against
  phpVMS `/api/user` and provisions MQTT creds. **By design public**: the
  api_key itself is the authentication factor.
- `POST /api/flight-logs/upload` — uses HTTP Basic-Auth (above).
- `POST /api/forms/aircraft` — uses `X-Forms-Token` header
  (`server.ts:1122-1144`), shared secret in DB.
- `GET /api/admin/push/vapid-public-key` (`server.ts:1012`) — returns only
  the **public** VAPID key. OK by design.

**Admin-cookie required:** every other `/api/*` route uses
`{ preHandler: requireAuth }`. WebSocket `/api/live` also has it
(`server.ts:1354`). Spot-checked all ~50 routes — no missed `requireAuth`.

### Geprüft: Auth-Bypass — keine Findings auf Logic-Ebene.
`requireAuth` consistently used. `getSession` checks expiry. No "skip
auth in dev" toggle that I could find.

### Pilot-Side Endpoints (Pilot-Client → phpVMS)
- All phpVMS calls use `X-API-Key` header
  (`crates/api-client/src/lib.rs:1009, 1080, 1112, 1128, 1180`).
- No pilot-client endpoint bypasses the API key.

### H1 — Login / provisioning have no rate limit
- **Severity:** High
- **Location:** `server.ts:132` (`/api/login`), `server.ts:112`
  (`/api/provision`)
- **Was passiert:**
  - `/api/login`: unlimited bcrypt-compares. Bcrypt is expensive (rounds=12
    ≈ 200 ms), so brute-force is naturally slow, but: (a) no IP-based
    rate limit means an attacker can saturate CPU and starve legitimate
    admins (DoS); (b) no account-lockout means weak passwords WILL fall
    to dictionary attacks given enough time.
  - `/api/provision`: each request makes a `fetch` to phpVMS `/api/user`.
    Attacker can use AeroACARS as a phpVMS-API-key oracle: try random
    keys, see which return 200 vs 401. Or just amplify DoS against
    phpVMS by submitting many keys.
  - Also: `/api/forms/aircraft` — token brute-force via shared secret in
    header.
- **Empfehlung:** Install `@fastify/rate-limit`. Apply globally to all
  `POST /api/*` at e.g. 30/min/IP, and a tighter 5/min/IP on `/api/login`,
  `/api/provision`, `/api/forms/aircraft`. Optionally fail2ban-on-Caddy
  logs for failed-login bursts.

### Info — Bcrypt rounds = 12
- `auth.ts:7`. Good (industry default 10-12). No action.

### Low — Session storage is in-memory `Map`
- `auth.ts:17`. On recorder restart all sessions are wiped — admins get
  re-prompted. Not a security issue (fail-secure), but means session
  fixation across multi-instance deployments is impossible (single-process
  is fine). Documented assumption. No action.

---

## 4. Input-Validation / SQL-Injection / Path-Traversal

### SQL queries
- All non-trivial DB queries use `better-sqlite3.prepare(...)` with `?`-
  parameters or `@name`-bindings (`db.ts` throughout, e.g.:
  `db.ts:1238-1240`, `db.ts:3113-3139` `searchAirports`).
- **Dynamic table/column construction reviewed:**
  - `db.ts:1218,1260` `UPDATE flight_sessions SET ${sets.join(", ")}` —
    `sets` is built from a hard-coded `allowed` whitelist
    (`["callsign","dep","arr","aircraft_icao"]`). Safe.
  - `db.ts:1527` `DELETE FROM ... WHERE id IN (${placeholders})` —
    `placeholders` is `"?,?,?"` pattern, values bound via spread. Safe.
- **Geprüft: SQL-Injection — keine Findings.**

### Input validation (Zod schemas)
- All POST bodies use `z.object({...}).safeParse(req.body)`. Sane bounds
  (`username.min(1).max(64)`, `password.min(1).max(256)`).
- Query parameters use `clamp(Number(...), min, max)` defensively in
  every range/limit consumer (`server.ts:191,377-378,413,422,...`). Good.

### Path-Traversal — Flight-Log-Upload (server.ts:578-675)
- **Sanitisation:** `safe = (s) => s.replace(/[^A-Za-z0-9_-]/g, "_")`
  applied to `va_prefix`, `pilot_id`, `pirep_id` before `join(FLIGHT_LOGS_ROOT, …)`.
- Authorization: `db.findSessionByPirepForPilot` enforces the pirep
  belongs to the authenticated pilot.
- Gzip magic-byte check before write.
- **OK** — defense in depth, no traversal vector I can construct.

### Path-Traversal — JSONL-Import (server.ts:819-844)
- `abs = resolve(file_path); if (!abs.startsWith(resolve(FLIGHT_LOGS_DIR)))` —
  correct prefix check.
- **Low concern:** on Windows the literal `FLIGHT_LOGS_DIR` is
  `/var/lib/aeroacars-recorder/flight-logs`. Cross-platform behavior of
  `resolve()` could produce surprising paths in a Windows dev env, but
  in prod (Linux VPS) this is fine.

### M1 — `/api/admin/jsonl-files` uses absolute server path
- **Severity:** Low
- `server.ts:747`: `FLIGHT_LOGS_DIR = "/var/lib/aeroacars-recorder/flight-logs"`
  hardcoded. If the dbPath/datadir is ever moved (e.g. ENV-overridden in
  config.ts) this endpoint silently breaks. **Not a security bug** —
  brittle constant. Note for refactor.

### Geprüft: Path-Traversal in `/api/pireps/:id/jsonl` — OK.
`sanitize()` applied even though the pirep_id comes from DB, double-defense.
(`server.ts:1226-1234`).

---

## 5. CORS / Security-Headers

### M2 — Caddy sends no security headers
- **Severity:** Medium
- **Location:** `aeroacars-live/vps/caddy/Caddyfile` (entire site block)
- **What's missing:**
  - `Strict-Transport-Security` (HSTS) — Caddy auto-enables this only
    on TLS sites with a domain, but the Caddyfile here doesn't include
    an explicit `max-age` and `includeSubDomains`. Manual setting
    recommended for clarity.
  - `Content-Security-Policy` — no CSP. The Webapp is a Vite-built React
    SPA served via `fastify-static`. Without CSP, any XSS (e.g. via an
    admin-uploaded JSONL containing crafted markdown rendered somewhere)
    can call `fetch("/api/...")` and exfiltrate everything.
  - `X-Frame-Options: DENY` / `frame-ancestors 'none'` — admin UI can
    be iframed by any site; click-jacking risk on `/admin/`.
  - `X-Content-Type-Options: nosniff` — missing.
  - `Referrer-Policy: strict-origin-when-cross-origin` — missing.
- **Empfehlung:** Add to Caddyfile inside the site block:
  ```
  header {
      Strict-Transport-Security "max-age=31536000; includeSubDomains"
      X-Content-Type-Options "nosniff"
      X-Frame-Options "DENY"
      Referrer-Policy "strict-origin-when-cross-origin"
      Content-Security-Policy "default-src 'self'; img-src 'self' data: https:; style-src 'self' 'unsafe-inline'; connect-src 'self' wss://live.kant.ovh https://aviationweather.gov; frame-ancestors 'none'"
  }
  ```
  Tighten CSP after a smoke-test of the actual webapp dependencies (Leaflet
  CDN tiles? OpenAIP?).

### Geprüft: CORS — keine Findings.
Fastify default = no CORS. Webapp is same-origin (`/admin/` served by same
Fastify instance). No `@fastify/cors` import. OK.

### M3 — Tauri webview has `csp: null`
- **Severity:** Medium
- **Location:** `CloudeAcars/client/src-tauri/tauri.conf.json:26-27`
- Disables Tauri's default CSP for the webview. Any future bug that
  injects HTML into the React tree (e.g. unescaped PIREP comment from
  phpVMS rendered as innerHTML) can run JS in the webview, which has
  access to all `#[tauri::command]` invocations — including
  `phpvms_login`, `store_api_key`, etc.
- **Empfehlung:** Set at least `"csp": "default-src 'self'; img-src
  'self' data: https:; style-src 'self' 'unsafe-inline'; connect-src
  'self' https://german-sky-group.eu https://www.simbrief.com
  https://aviationweather.gov wss://live.kant.ovh"`. Test thoroughly on
  every API call before shipping.

---

## 6. MQTT-Security

### Mosquitto config
- **`allow_anonymous false`** + **`password_file`** + **`acl_file`**
  (`mosquitto.conf:17-19`). Auth enforced.
- Both listeners (1883 plain MQTT, 1884 plain WS) bound to **127.0.0.1
  only** (`mosquitto.conf:39-44`). Public traffic goes through Caddy on
  443 → `/mqtt` → 127.0.0.1:1884 (`Caddyfile:29-31`). Good.

### ACLs — per-pilot topic isolation
- Pilot user `pilot_<id>` gets `readwrite aeroacars/<va>/<id>/#`
  (`bootstrap.sh:163-166`, `acl.conf.example:11`).
- Monitor user has read-only on `aeroacars/#` (`acl.conf.example:7`).
- **Pilot CANNOT subscribe to or publish other pilots' topics.** Good.

### M4 — Pilot MQTT passwords stored plaintext in `provisioned_pilots`
- **Severity:** Medium
- **Location:** `aeroacars-live/recorder/src/db.ts:269-280` (schema),
  `provision.ts:97-100` (upsert), `server.ts:609-611` (Basic-Auth check
  via SHA-256 normalisation, not bcrypt).
- **Why this is necessary by design:**
  - Re-provisioning (cache hit, `provision.ts:79-91`) must return the
    same password to the pilot client so they don't have to re-pair.
  - `mosquitto_passwd` stores its own bcrypt hash for runtime auth, so
    even if the recorder DB is dumped, the broker itself is independent.
- **Was passiert wenn ausgenutzt:** Anyone with read access to the
  SQLite DB (= local file on VPS, or via a webapp SQL-injection if one
  existed, or via a hypothetical export endpoint) gets all pilot
  passwords plaintext. They can then connect as those pilots over MQTT
  and publish fake position/touchdown/PIREP events, polluting the
  database. **Bounded blast radius**: pilot-cred = can-publish-bullshit-
  for-that-pilot. Cannot read other pilots' data (ACL).
- **Empfehlung:** Document explicitly in `db.ts` and the architecture
  doc that pilot MQTT passwords are stored plaintext **on purpose** and
  what the threat model is. Consider encrypting `password` column at
  rest with a key derived from `SESSION_SECRET` if/when paranoia
  warrants it. Lock down filesystem permissions on `aeroacars-live.db`
  (currently inherits whatever systemd unit sets — verify mode 0600
  owned by `aeroacars`).

### Geprüft: WSS/TLS — OK.
Pilot client connects to `wss://live.kant.ovh/mqtt`. TLS terminated at
Caddy. Mosquitto-to-Caddy hop is plain on loopback. No MITM on the wire.

### Info — `MQTT_USERNAME` / `MQTT_PASSWORD` for the recorder are in
`/etc/aeroacars-recorder.env` (systemd EnvironmentFile). Standard practice.

---

## 7. Keyring / Credential-Storage im Pilot-Client

### M5 — phpVMS API key + MQTT creds stored as plaintext JSON, NOT in OS keyring
- **Severity:** Medium (CONTRADICTS user-memory)
- **Location:** `CloudeAcars/client/src-tauri/crates/secrets/src/lib.rs` (whole file)
- **What:** Single JSON file at `<app_data_dir>/secrets.json`:
  - Unix: `chmod 0600`, owner-only (`lib.rs:139-146`)
  - Windows: relies on default ACL inheritance of `%APPDATA%\com.aeroacars.app\` (`lib.rs:148-154`)
- **User-Memory says:** *"phpVMS-API-Key wird via Windows Credential
  Manager / macOS Keychain gespeichert (NICHT in DB!)"* —
  **this is no longer accurate**.
  The code (with extensive comments at `lib.rs:1-39`) deliberately moved
  away from `keyring` in v0.5.15 because of macOS Keychain prompt-loop
  on every unsigned ad-hoc-signed build. The `migrate_from_keyring`
  function (`lib.rs:195-239`) handles one-shot migration on startup.
- **Was passiert wenn ausgenutzt:** Any process running as the same user
  (= any Windows malware that already got code-exec in the user account)
  can read `secrets.json` directly without UAC prompt or Keychain ACL
  challenge. With Keychain/Credential-Manager, an attacker would at
  least face the OS-level ACL barrier (though as the code comment
  correctly notes, process injection bypasses that too).
- **Risk in practice:** **About the same as smartCARS/vmsACARS/FsAcars**
  (per the module docstring's own honest disclaimer). Not a regression
  vs. the VA-pilot-tool industry standard, but is a regression vs. the
  user's mental model.
- **Empfehlung:**
  1. **Update `MEMORY.md`** to reflect file-based storage (not keyring).
  2. The current mitigation (0600 perms on Unix, %APPDATA% ACL on
     Windows) is the practical maximum for an unsigned app. If
     Apple/Windows code-signing becomes available, revisit and
     consider re-adding Keychain/Credential-Manager backends for those
     code-signed builds (where the "always allow" actually sticks).
  3. Consider AES-encrypting the JSON file with a key derived from a
     machine-bound secret (e.g. DPAPI on Windows,
     `security-framework`'s machine secret on macOS) — would defeat
     casual file-grab attacks without prompting UX hell.

### Info — Atomic write + `0o600` is correct
`write_store` writes tmp + rename + chmod 600. Idempotent and safe
against torn writes. No action.

---

## 8. Dependencies — known Vulns

### H3 — Recorder: `tar` chain via `bcrypt`'s `node-pre-gyp` (6 CVEs)
- **Severity:** High
- **Location:** `aeroacars-live/recorder/package-lock.json`
- `npm audit` output:
  ```
  tar  <7.5.7  (6 advisories incl. CVE GHSA-34x7-hfp2-rc4v CVSS 8.2)
  @mapbox/node-pre-gyp <=1.0.11 (depends on vulnerable tar)
  ```
  Six advisories: hardlink path-traversal, symlink poisoning,
  drive-relative linkpath, Unicode-ligature race on APFS, etc.
- **Why it matters here:** `bcrypt` uses `node-pre-gyp` at install time
  (postinstall script) to fetch native binaries. The vulnerable `tar`
  extraction runs only during `npm install` against tarballs from
  remote URLs. If the registry is compromised or a malicious mirror is
  injected, postinstall extraction could write to arbitrary paths.
- **Runtime impact: low** (no tar-extraction in serving code path).
- **Install-time impact: high** if an attacker controls the npm
  registry response.
- **Empfehlung:** Run `npm audit fix` (or pin newer `bcrypt`@^5.1.1
  pulls newer `node-pre-gyp@>=2`). Verified non-breaking change.
  Better alternative: switch to `bcryptjs` (pure JS, no `node-pre-gyp`).

### @fastify/static moderate (2 advisories)
- **Severity:** Medium
- `GHSA-pr96-94w5-mx2h` — path traversal in directory listing.
  AeroACARS does not enable directory listing, so the vulnerable code
  path isn't reached. Still: upgrade to `@fastify/static@>=9.1.3`.
- `GHSA-x428-ghpx-8j92` — encoded path separators bypass route guards.
  This one is more relevant since we use a custom `setHeaders` hook
  for cache-control on specific paths. Upgrade.

### Webapp
- `npm audit`: **0 vulnerabilities**. Clean.

### Pilot Client (`E:\CloudeAcars\client`)
- `npm audit`: 5 moderate, all in `esbuild`/`vite`/`vitest` dev-only
  chain. **No runtime exposure** (these are only in `devDependencies`,
  shipping bundle is the Tauri Rust binary + the bundled HTML/JS).
- **Empfehlung:** Bump vite/vitest at next dependency sweep, but not
  user-facing risk.

### Rust deps (manual review — `cargo-audit` not installed)
- `tokio = 1.x` — current.
- `reqwest = 0.12` — current major, `rustls-tls` (no OpenSSL surface).
- `rusqlite = 0.32 [bundled]` — bundled SQLite vendor patches applied
  by upstream. Generally fine; check next dep-sweep.
- `tauri = 2.x` — current major.
- **No known CVE I'm aware of as of 2026-05-12, but recommend
  installing `cargo-audit` in CI and running it on every PR.**

---

## 9. Update-Mechanism Security

### Updater config (Pilot client)
- **Endpoint:** `https://github.com/MANFahrer-GF/AeroACARS/releases/latest/download/latest.json`
  (`tauri.conf.json:50`). HTTPS-only — Tauri-Updater rejects non-HTTPS,
  no fallback. Good.
- **Signature verification:** Pubkey ID `4D3A4751C0F98408` embedded
  base64 at `tauri.conf.json:52`. Tauri's updater verifies minisign
  signatures before applying. Good.
- **TLS:** uses system TLS via `reqwest`'s rustls feature transitively.
  Good.

### Threat model
- Compromise of GitHub-Release upload: attacker can publish a `.json`
  + binaries, but without the **private** minisign key (see H4) cannot
  produce a valid signature → updater rejects.
- Compromise of pilot's machine: attacker can replace `latest.json` in
  flight, but TLS to github.com + binary's pubkey-embedded check makes
  this hard without breaking TLS and the signature.
- **Single point of failure:** the private signing key (H4). Everything
  else is solid.

### Info — Updater pubkey base64 is `4D3A4751C0F98408` formatted
Decoded: `untrusted comment: minisign public key: 4D3A4751C0F98408`
followed by the base64-encoded raw pubkey. Matches the `.pub` file in
the working tree. Consistent.

---

## 10. Logs / Telemetry-Leaks

### Pilot client logs (`tracing::*`)
- **Geprüft via grep:** `tracing::(info|debug|warn|error)!.*api_key`,
  `.*password`, `.*token`. **Zero matches with secret values.**
- Only one hit: `lib.rs:10128` — `tracing::warn!("log-upload: no MQTT
  password in keyring — skip")`. Logs the *absence*, not the value.
  Good.
- Account names (`KEYRING_ACCOUNT`, `MQTT_KEYRING_PASSWORD`) are
  logged at debug level in `secrets::store_api_key` etc.
  (`secrets/lib.rs:165, 178, 232`). Account names are constants
  (`"primary"`, `"mqtt-password"`, …), not values. Safe.

### Recorder logs
- Fastify default logger is set with `logger: true` (`server.ts:39`).
- **Body logging is OFF by default** in Fastify v5 — request bodies are
  NOT logged unless explicitly added to `serializers`. Spot-checked:
  no custom body serializer. Good.
- **Concern:** `console.warn` in `provision.ts:45,49,117` logs error
  context but not the actual API key. Verified — only the URL and
  status code or error message hit logs.

### SQLite payloads
- `db.ts` stores `payload_json` for touchdowns/pireps/positions. These
  payloads come from MQTT-published telemetry, which is **flight data,
  not credentials**. Auditor's pilot-client side never sends the phpVMS
  API key over MQTT — it sends it as `X-API-Key` header to phpVMS
  directly. Confirmed via grep over `crates/aeroacars-mqtt/`.
- **Geprüft:** API keys do NOT land in the `aeroacars-live.db`
  payloads. Clean.

### Discord embeds
- `recorder/src/discord.ts:42-60` and `client/src-tauri/src/discord.rs`
  build embeds with callsign, dep/arr, score, V/S etc. No credentials,
  no PII beyond pilot ident which is already public in the VA.

---

## Appendix A — VPS root-escalation paths
(Detailed analysis of finding C2)

`bootstrap.sh:67`:
```bash
echo "$APP_USER ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/$APP_USER
```
This grants the `aeroacars` system user **passwordless full sudo**. The
recorder service runs as `aeroacars` (`aeroacars-recorder.service:9-10`).

The recorder process is reachable via the admin webapp (cookie-auth):
- `POST /api/admin/updates/install` → `sudo apt-get -y upgrade` (server.ts:935-972)
- `POST /api/admin/system/reboot` → `sudo shutdown -r +1` (server.ts:980-999)
- `POST /api/pilots` → `sudo /usr/local/bin/aeroacars-add-pilot ...` (server.ts:1063-1072)

**The intended design** is encoded in `sudoers.d-aeroacars`: a tight
whitelist of allowed commands (mosquitto_passwd, sed on acl.conf,
systemctl reload mosquitto, cat passwords). That file is the right
model.

**The actual deployment** is wider than intended because `bootstrap.sh`
unconditionally writes the `NOPASSWD:ALL` line BEFORE the surgical
`sudoers.d-aeroacars` would be installed by `deploy-recorder.sh`. So
`/etc/sudoers.d/aeroacars` (from bootstrap, ALL) and
`/etc/sudoers.d/aeroacars-recorder` (surgical) co-exist, and `ALL`
wins for any command not listed in the surgical file.

**Was passiert wenn ausgenutzt:**
- One stolen admin session cookie (e.g. via XSS, see M2/M3) →
  attacker calls `/api/admin/updates/install`. apt-get runs as root,
  with `DEBIAN_FRONTEND=noninteractive` and `Dpkg::Options::=--force-confold`.
- Or simpler: any RCE in the Node recorder process (e.g. via a future
  body-parsing flaw, dependency CVE) → spawn `sudo bash` → root.
- Or: the existing endpoint composition. There's no command-injection
  in the implemented `execSync` calls (no user input in command
  strings — the bodies use Zod literals like `confirm: "REBOOT"`).
  But the **next** added endpoint that takes user input and shells
  out gets `NOPASSWD:ALL` for free.

**Empfehlung:**
1. Remove the line `echo "$APP_USER ALL=(ALL) NOPASSWD:ALL"` from
   `bootstrap.sh:67`. Rely entirely on the surgical
   `sudoers.d-aeroacars` whitelist.
2. **Add** the additional commands the recorder actually needs (so the
   webapp endpoints keep working with least-privilege):
   ```
   aeroacars ALL=(root) NOPASSWD: /usr/bin/apt-get update -qq
   aeroacars ALL=(root) NOPASSWD: /usr/bin/apt-get -y -o * upgrade
   aeroacars ALL=(root) NOPASSWD: /usr/bin/apt list --upgradable
   aeroacars ALL=(root) NOPASSWD: /usr/sbin/shutdown -r +1 *
   aeroacars ALL=(root) NOPASSWD: /usr/sbin/shutdown -c
   ```
   (Verify exact paths with `which apt-get`/`which shutdown` on the
   target Debian 12.)
3. Document in `vps/README.md` that the recorder **should never run as
   root** and that any future sudo-requiring command must go through
   the whitelist.

---

## Appendix B — Files reviewed

**aeroacars-live:**
- `recorder/src/server.ts` (1408 lines, all endpoints)
- `recorder/src/auth.ts`
- `recorder/src/config.ts`
- `recorder/src/provision.ts`
- `recorder/src/pilotMgmt.ts`
- `recorder/src/discord.ts` (first 50 lines)
- `recorder/src/db.ts` (schemas + dynamic-SQL spots)
- `recorder/src/mqttSubscriber.ts` (first 80 lines)
- `recorder/package.json` + `npm audit`
- `webapp/` `npm audit` (clean)
- `vps/caddy/Caddyfile`
- `vps/mosquitto/mosquitto.conf`
- `vps/mosquitto/acl.conf.example`
- `vps/bootstrap.sh`
- `vps/sudoers.d-aeroacars`
- `vps/systemd/aeroacars-recorder.service`

**CloudeAcars (Pilot-Client):**
- `client/src-tauri/tauri.conf.json`
- `client/src-tauri/src/discord.rs`
- `client/src-tauri/src/lib.rs` (auth + provisioning sections,
  upload section)
- `client/src-tauri/crates/secrets/src/lib.rs`
- `client/src-tauri/crates/api-client/src/lib.rs` (header grep)
- `client/aeroacars-updater.key` + `.pub` (file presence/encoding only)
- `client/src-tauri/Cargo.toml` (versions of major deps)
- `.gitignore` (verified `*.key` rule active)
- `client/` `npm audit` (5 moderate, all dev-only)

---

## Appendix C — Suggested fix priority

1. **NOW** (this week):
   - Rotate Discord webhook (C1).
   - Move `aeroacars-updater.key` out of repo tree (H4).
   - Remove `bootstrap.sh:67` `NOPASSWD:ALL` line + extend
     `sudoers.d-aeroacars` (C2).
2. **SOON** (this month):
   - `npm audit fix` on recorder (H3) + bump `@fastify/static`.
   - Install `@fastify/rate-limit`, apply to login + provision (H1).
   - Add Caddy security headers (M2).
   - Set Tauri webview CSP (M3).
   - Update `MEMORY.md` to reflect file-based secrets (M5
     documentation gap).
3. **LATER** (next release cycle):
   - Confirm `/etc/aeroacars-recorder.env` permissions (0600).
   - Consider encrypting `provisioned_pilots.password` column (M4).
   - Install `cargo-audit` in CI.
   - Document update-key rotation procedure (H4 follow-up).
