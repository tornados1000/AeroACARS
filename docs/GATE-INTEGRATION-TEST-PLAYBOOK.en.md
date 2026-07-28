# AeroACARS Integrity Gate ↔ VPS Recorder — Integration Test Playbook

**Spec:** v0.13.0  
**Repos:**
- VPS Recorder: `aeroacars-live` (Hetzner, live.kant.ovh)
- Gate module: `aeroacars-integrity-gate` (private, MANFahrer-GF)
- phpVMS host: `german-sky-group.eu`

---

## Overview

The module lives on the phpVMS host and exchanges HMAC-signed calls with the recorder on the VPS:

```
                         ┌──────────────────────┐
                         │  PILOT CLIENT (Tauri) │
                         └──────┬───────────────┘
                                │ MQTT positions+pirep
                                ▼
       ┌────────────────────────────────────────┐
       │     VPS RECORDER (live.kant.ovh)        │
       │  - integrityValidator                   │
       │  - scoreTrust                           │
       │  - /api/integrity-check/:pirep_id       │◄────┐
       │  - /api/integrity-check/:id/gate-ack    │     │ HMAC
       │  - /api/admin/pireps/:id/review         │     │
       └─────────────────────┬───────────────────┘     │
                             │ HMAC out                │
                             │ webhook decision        │
                             ▼                         │
       ┌────────────────────────────────────────┐     │
       │   PHPVMS HOST (german-sky-group.eu)     │     │
       │  - PirepFiled event                     │     │
       │  - AeroACARSIntegrityGate module        │─────┘
       │  - /api/aeroacars-gate/webhook/decision │
       │  - /admin/aeroacars-gate (review UI)    │
       └────────────────────────────────────────┘
```

---

## Phase 1 — Deployment preparation

### 1.1 Install the gate module on the phpVMS host

On `german-sky-group.eu` (or test environment):

```bash
cd /var/www/phpvms  # or wherever phpVMS lives
cd modules/  # if the modules directory exists; otherwise via nwidart/laravel-modules
git clone git@github.com:MANFahrer-GF/aeroacars-integrity-gate.git AeroACARSIntegrityGate
cd AeroACARSIntegrityGate
composer install --no-dev
cd ../..

# Run migrations
php artisan migrate --path=modules/AeroACARSIntegrityGate/database/migrations

# Enable the module (convention)
php artisan module:enable AeroACARSIntegrityGate

# Publish config
php artisan vendor:publish --tag=aeroacars-integrity-gate-config

# Clear caches
php artisan config:clear && php artisan cache:clear && php artisan route:clear
```

### 1.2 Generate shared secret + set on BOTH hosts

```bash
# Generate once — e.g. on the VPS
openssl rand -hex 32
# → copy the output, e.g.:
#   3f8e9a2b4c5d6e7f8901a2b3c4d5e6f7890a1b2c3d4e5f60718293a4b5c6d7e8
```

**On the phpVMS host (`/var/www/phpvms/.env`):**
```
AEROACARS_GATE_VPS_URL=https://live.kant.ovh
AEROACARS_GATE_SHARED_SECRET=3f8e9a2b4c5d6e7f8901a2b3c4d5e6f7890a1b2c3d4e5f60718293a4b5c6d7e8
AEROACARS_GATE_DISCORD_WEBHOOK=https://discord.com/api/webhooks/...  # optional
AEROACARS_GATE_AUTO_REPAIR_ON_BOOT=true
```

**On the VPS (`/etc/aeroacars-recorder/env` or via systemd unit):**
```
AEROACARS_GATE_BASE_URL=https://german-sky-group.eu
AEROACARS_GATE_SHARED_SECRET=3f8e9a2b4c5d6e7f8901a2b3c4d5e6f7890a1b2c3d4e5f60718293a4b5c6d7e8
```

phpVMS cache + service restart:
```bash
# phpVMS host
php artisan config:cache

# VPS
sudo systemctl restart aeroacars-recorder
```

### 1.3 Run auto-repair for all ranks initially

On the phpVMS host:
```bash
php artisan aeroacars:integrity-auto-repair-check
# Expected output if drift: "Auto-repaired X rank(s) where ..."
# If already clean: "All ranks clean."
```

In the phpVMS admin: open `/admin/aeroacars-gate` → "Gate safety status" must be **green**.

---

## Phase 2 — Connectivity smoke tests from the VPS

### 2.1 Automated test via CLI

On the VPS:
```bash
cd /opt/aeroacars-live/recorder
# Source the service ENV so AEROACARS_GATE_SHARED_SECRET is available
export $(systemctl show aeroacars-recorder -p Environment | tr ' ' '\n' | grep AEROACARS_GATE_SHARED_SECRET | sed 's/Environment=//')
# Or export manually

npm run test-gate-integration https://german-sky-group.eu
```

**Expected:** 5 tests green:
1. ✓ gate base reachable (HTTP 200/404 — TLS + DNS OK)
2. ✓ webhook rejects unsigned request (HTTP 401)
3. ✓ webhook rejects bad HMAC (HTTP 401)
4. ✓ signed webhook passed HMAC, rejected on validation (HTTP 422 / 404 — HMAC OK, pirep_id=0 invalid)
5. ✓ admin status endpoint exists + auth-gated (HTTP 302/401/403)

**Error diagnosis:**

| Test 4 → HTTP 401 | HMAC SECRET MISMATCH | Compare both ENVs, are they exactly the same? |
| Test 1 → unreachable | DNS or firewall | `curl -v https://german-sky-group.eu` |
| Test 2 → HTTP 200 | Webhook not HMAC-verified | Code bug, check webhook controller |
| Test 4 → HTTP 500 | Server error in phpVMS | Laravel log: `tail -f storage/logs/laravel.log` |

### 2.2 Manual connectivity probe from the admin UI

In the phpVMS browser:
1. Log in as admin
2. Open `/admin/aeroacars-gate`
3. "Test connection" button with PIREP ID `1`
4. Expected green box: *"VPS responded with no_telemetry (404) for pirep_id=1 — connection works, this pirep_id has no recorder data."*

---

## Phase 3 — End-to-end with a real test flight

### 3.1 Clean flight (verdict=clean)

1. **Pilot**: Start the sim, open the AeroACARS client, pick a bid (e.g. GSG123 EDDF→LOWW), fly normally.
2. **File the PIREP** via the AeroACARS client.
3. **Watch the recorder log** (VPS):
   ```
   journalctl -u aeroacars-recorder -f | grep integrity
   ```
   Expected: `[integrity] gate-client configured...`, then on PIREP submit: score_trust_level=trusted.

4. **Watch the phpVMS log** (phpVMS host):
   ```
   tail -f storage/logs/laravel.log | grep AeroACARS-Gate
   ```
   Expected:
   - `AeroACARS-Gate per-PIREP auto-repair` (only if drift)
   - `AeroACARS-Gate verdict applied (in-flow) ... verdict=clean`
   - NO exception/error logs

5. **phpVMS PIREP list** (`/admin/pireps`):
   - PIREP state: `ACCEPTED`
   - PIREP status: phpVMS default (not `INT_HOLD_*`)

### 3.2 Suspicious flight (verdict=untrusted)

Simulate an anomaly:
- Slew mode ON during cruise (= position delta flag)
- OR: sim pause + fuel reset (= FUEL_RATE + SIM_STATE_RESET_SIGNATURE flags)

1. Flight + PIREP file as above.
2. **Recorder log:** Validator flags are generated, score_trust_level=untrusted.
3. **phpVMS log:**
   ```
   AeroACARS-Gate verdict applied (in-flow) ... verdict=untrusted
   ```
4. **phpVMS PIREP:**
   - State: `PENDING` (NOT ACCEPTED!)
   - Status: `INT_HOLD_UNTR`
5. **Gate review UI** (`/admin/aeroacars-gate/review`):
   - PIREP appears with `held_untrusted` badge
   - Reasons visible (hard_trigger_gs_zero, etc.)
   - Flag types listed

### 3.3 Admin decision roundtrip

1. **In the AeroACARS-Live web app** (`https://live.kant.ovh/admin/#/review`):
   - Select the PIREP from the review queue
   - Click "Reject" with reason "Sim crash recovered"
2. **Recorder log:**
   ```
   [integrity] gate-queue drained {attempted: 1, succeeded: 1}
   ```
3. **phpVMS log:**
   ```
   AeroACARS-Gate webhook: decision applied
     decision_id=dec-XXX target_state=rejected reviewer=admin
   ```
4. **phpVMS PIREP:**
   - State: `REJECTED`
   - Lifecycle events fired (Discord post if configured)
5. **Gate review UI:**
   - PIREP disappears from the default queue
   - Visible with `?history=1` with decision info
6. **Recorder pireps row:**
   - `gate_sync_status='synced'`
   - `gate_ack_state='admin_decision'`
   - `decision_committed_at` set

---

## Phase 4 — Failure mode tests (chaos engineering)

### 4.1 Gate temporarily down

```bash
# On the phpVMS host
sudo systemctl stop nginx  # or apache2
```

1. File a PIREP (should work — the recorder is not affected)
2. **Recorder log:** Verdict call fails → `gate_outbound_queue` enqueued
3. Bring the service back up: `sudo systemctl start nginx`
4. Wait max. 5 min (drainer interval)
5. **Recorder log:** `[integrity] gate-queue drained {succeeded: N}`
6. **phpVMS PIREP:** Lifecycle fired retroactively

### 4.2 Recorder temporarily down

```bash
# On the VPS
sudo systemctl stop aeroacars-recorder
```

1. File a PIREP via the phpVMS API
2. **phpVMS log:** `AeroACARS-Gate: VPS unreachable, PENDING-Hold`
3. **phpVMS PIREP:** State `PENDING`, status `INT_HOLD_TIMEOUT`
4. Restart the recorder: `sudo systemctl start aeroacars-recorder`
5. Wait max. 5 min (gate AsyncVerdictRetryQueue)
6. **phpVMS log:** `AeroACARS-Gate verdict applied (off-flow)`
7. **phpVMS PIREP:** State corrected per verdict (ACCEPTED if clean, remains PENDING if review)

### 4.3 HMAC secret mismatch (negative test)

```bash
# On the phpVMS host only — set the secret wrong
echo "AEROACARS_GATE_SHARED_SECRET=wrong-secret-for-testing-only" >> .env.test
php artisan config:cache
```

1. File a PIREP
2. **phpVMS log:** `AeroACARS-Gate webhook: hmac_invalid`
3. ALL PIREPs land in `INT_HOLD_ERROR` because the verdict call returns 401
4. Fix the secret + config:cache, then new PIREPs work again
5. Old PIREPs must be decided manually via the admin web app review

---

## Phase 5 — Production health monitoring

Regularly check during operation:

### On the phpVMS host
```bash
# Status JSON (requires admin cookie via curl --cookie)
curl https://german-sky-group.eu/admin/aeroacars-gate/status.json | jq

# Expected indicators:
#   auto_repair.gate_safe: true                  ← MUST be true
#   auto_repair.unsafe_count: 0                  ← MUST be 0
#   retry_queue.abandoned: < 5                   ← if > 0, investigate
#   callback_queue.abandoned: < 5                ← ditto
#   claims.in_progress: < 10                     ← if high, worker crash?
```

### On the VPS
```bash
# Recorder-side gate queue status
curl -H "Authorization: Bearer $ADMIN_TOKEN" https://live.kant.ovh/api/admin/integrity/gate-queue/status | jq

# Expected indicators:
#   pending: < 10
#   abandoned: < 5
```

### Daily cron verification

The phpVMS cron tab must contain:
```cron
0 4 * * * cd /var/www/phpvms && php artisan schedule:run >> /dev/null 2>&1
```

In the schedule (`app/Console/Kernel.php`):
```php
$schedule->command('aeroacars:integrity-auto-repair-check')->dailyAt('04:00');
$schedule->command('aeroacars:integrity-retry-queue-drain')->everyFiveMinutes();
$schedule->command('aeroacars:integrity-recorder-callback-drain')->everyTwoMinutes();
```

---

## Troubleshooting cheat sheet

| Symptom | Probable cause | Fix |
|---|---|---|
| All PIREPs land in `INT_HOLD_TIMEOUT` | Recorder unreachable from phpVMS host | `curl https://live.kant.ovh` from phpVMS, check firewall |
| All PIREPs in `INT_HOLD_ERROR` | HMAC SECRET MISMATCH | Check both ENVs, are they exactly the same? `config:cache` after change |
| All PIREPs immediately ACCEPTED | `auto_approve_acars=true` AND auto-repair down | Run `aeroacars:integrity-auto-repair-check` manually, then investigate the Discord alert |
| `INT_HOLD_PENDING` stuck | not_ready loop, TD event never arrived | Check pilot client log, look at session last_seen |
| `INT_REJ_REPAIR_FAIL` | AutoRepair failure (DB lock, plugin conflict) | Laravel log, manual DB correction, then `php artisan aeroacars:integrity-claim-cleanup` |
| 409 `claim_incomplete` from webhook | Worker hung or crashed | `ps aux \| grep queue:work`, then `aeroacars:integrity-claim-cleanup --decision-id=...` if needed |
| Recorder review tab inconsistent with phpVMS | Callback queue abandoned | Run `aeroacars:integrity-recorder-callback-drain --limit=200` manually |
