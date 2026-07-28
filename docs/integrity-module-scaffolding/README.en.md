# AeroACARS Integrity Gate — phpVMS Module Scaffolding (Slice 7)

**Status:** Scaffolding-only bootstrap skeleton. Do NOT commit this into this repo — copy it into your **own repository** (e.g. `MANFahrer-GF/aeroacars-integrity-gate`).

**Spec:** [`docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md`](../spec/v0.13.0-mid-session-integrity-and-resume-policy.md) — Stream G (LE29–LE34).

---

## What this skeleton contains

This directory structure matches 1:1 the layout a phpVMS module must have per phpVMS convention (see `phpVMS/Modules/*` for official examples and [DisposableBasic](https://github.com/FatihKoz/DisposableBasic) for the reference pattern).

```
aeroacars-integrity-gate/
├── composer.json                    # Module metadata + autoload
├── module.json                      # phpVMS module manifest
├── README.md
├── app/
│   ├── AeroACARSIntegrityGateServiceProvider.php
│   ├── Listeners/
│   │   └── PirepFiledGate.php       # ← LE30 (code in spec)
│   ├── Services/
│   │   ├── VpsClient.php            # ← LE31 (HMAC auth, single-attempt)
│   │   ├── HmacSigner.php           # ← LE31a (HMAC-SHA256 + nonce + TTL)
│   │   ├── AutoRepairService.php    # ← LE30 G.2.0 (boot + daily + per-PIREP)
│   │   ├── PirepStateManipulator.php # ← LE33 (applyVerdict + fallbacks)
│   │   └── AsyncVerdictRetryQueue.php # ← LE33 async retry worker
│   ├── Http/Controllers/
│   │   └── IntegrityWebhookController.php # ← LE34 (webhook for admin decisions)
│   ├── Exceptions/
│   │   ├── VpsUnreachableException.php
│   │   ├── VpsNoTelemetryException.php
│   │   └── AutoRepairVerificationFailedException.php
│   └── Console/Commands/
│       ├── IntegrityAutoRepairCheck.php # daily cron
│       └── IntegrityRetryQueueDrain.php # async retry worker
├── database/migrations/
│   ├── 2026_05_24_000001_create_pirep_review_metadata.php
│   └── 2026_05_24_000002_create_module_webhook_retry_queue.php
├── config/
│   └── aeroacars-integrity-gate.php
├── resources/
│   ├── views/admin/settings.blade.php
│   └── lang/{de,en}/messages.php
├── routes/
│   └── web.php                      # Webhook + admin settings routes
└── tests/
    ├── Feature/PirepFiledGateTest.php
    ├── Feature/WebhookControllerTest.php
    └── Unit/HmacSignerTest.php
```

---

## Bootstrap steps (for the new repo)

1. Create a new repository: `gh repo create MANFahrer-GF/aeroacars-integrity-gate --private`.
2. `git clone` it locally.
3. **Copy the contents of this scaffolding directory** into the new repo root.
4. `composer install` locally with `phpunit/phpunit ^10` + `nwidart/laravel-modules` dev-deps.
5. `php artisan module:make AeroACARSIntegrityGate` if you want to follow the phpVMS CLI convention, then merge in the contents.
6. Set module settings + HMAC secret in `config/aeroacars-integrity-gate.php`.
7. **IMPORTANT (Spec R8):** On the GSG side, set `auto_approve_acars = FALSE` on every rank — otherwise the PENDING hold won't take effect. See the LE30 setup guide.
8. Run the migration: `php artisan migrate --path=Modules/AeroACARSIntegrityGate/database/migrations`.
9. Enable the module: `php artisan module:enable AeroACARSIntegrityGate`.
10. Tests: `vendor/bin/phpunit Modules/AeroACARSIntegrityGate/tests`.

---

## Security constraints (carried over from the spec)

- **HMAC-SHA256** (module ↔ VPS) with `timestamp + nonce + TTL=300s + replay cache` (LE31a)
- **Shared secret** as an ENV variable, never in the DB
- **Fail-closed** to PENDING hold on EVERY failure (VPS timeout, 404, Throwable, config drift) — LE33
- **Auto-repair pattern**: boot + daily cron + per-PIREP (LE30 G.2.0)
- **REJECTED emergency fallback** if auto-repair itself throws an exception — LE30 R13
- **Listener vs. webhook lifecycle split** — the listener uses direct Eloquent state manipulation (in-flow), the webhook uses `PirepService::accept()/reject()` (off-flow, lifecycle events). LE33.

---

## Not done in Slice 7 scaffolding

This directory provides **skeleton files with TODO markers**, not the finished implementations. The complete spec implementation belongs in the dedicated repo, and there:

- Full Laravel test suite with `RefreshDatabase` + phpVMS test helpers
- CI pipeline (GitHub Actions: phpunit + larastan)
- Composer package release workflow
- Module settings admin UI (Blade)
- DE/EN translations
- README with a setup guide for VAs (with the `auto_approve_acars=false` note prominently featured)

**Effort estimate for Slice 7 in the dedicated repo:** ~40h (Spec R6 Stream G).

---

## How to read the code stubs

Each stub file contains:
1. A PHPDoc header with a spec LE reference
2. A class stub with signature-correct methods
3. `// TODO(Slice 7 PR-N): see spec section X` markers at every implementation point
4. An inline doc block with the behavioral requirements from the spec

This lets an implementor (or a follow-up agent) fill in the stubs point by point against the spec template.
