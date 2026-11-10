# AeroACARS Integrity Gate — phpVMS Module Scaffolding (Slice 7)

**Status:** Scaffolding-Only Bootstrap-Skelett. NICHT in dieses Repo committen — kopieren in **eigenes Repository** (z. B. `MANFahrer-GF/aeroacars-integrity-gate`).

**Spec:** [`docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md`](../spec/v0.13.0-mid-session-integrity-and-resume-policy.md) — Stream G (LE29–LE34).

---

## Was dieses Skelett enthält

Diese Verzeichnis-Struktur entspricht 1:1 dem Layout das ein phpVMS-Modul nach phpVMS-Konvention haben muss (siehe `phpVMS/Modules/*` für offizielle Beispiele und [DisposableBasic](https://github.com/FatihKoz/DisposableBasic) für das Referenz-Pattern).

```
aeroacars-integrity-gate/
├── composer.json                    # Modul-Metadata + Autoload
├── module.json                      # phpVMS Module Manifest
├── README.md
├── app/
│   ├── AeroACARSIntegrityGateServiceProvider.php
│   ├── Listeners/
│   │   └── PirepFiledGate.php       # ← LE30 (Code im Spec)
│   ├── Services/
│   │   ├── VpsClient.php            # ← LE31 (HMAC-Auth, single-attempt)
│   │   ├── HmacSigner.php           # ← LE31a (HMAC-SHA256 + nonce + TTL)
│   │   ├── AutoRepairService.php    # ← LE30 G.2.0 (boot + daily + per-PIREP)
│   │   ├── PirepStateManipulator.php # ← LE33 (applyVerdict + Fallbacks)
│   │   └── AsyncVerdictRetryQueue.php # ← LE33 Async-Retry-Worker
│   ├── Http/Controllers/
│   │   └── IntegrityWebhookController.php # ← LE34 (Webhook für Admin-Decisions)
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
│   └── web.php                      # Webhook + Admin-Settings-Routes
└── tests/
    ├── Feature/PirepFiledGateTest.php
    ├── Feature/WebhookControllerTest.php
    └── Unit/HmacSignerTest.php
```

---

## Bootstrap-Schritte (für den neuen Repo)

1. Neues Repository anlegen: `gh repo create MANFahrer-GF/aeroacars-integrity-gate --private`.
2. `git clone` lokal.
3. **Kopiere den Inhalt von diesem Scaffolding-Verzeichnis** in den neuen Repo-Root.
4. `composer install` lokal mit `phpunit/phpunit ^10` + `nwidart/laravel-modules`-Devdeps.
5. `php artisan module:make AeroACARSIntegrityGate` falls man die phpVMS-CLI-Konvention nehmen will, dann den Inhalt mergen.
6. Module-Settings + HMAC-Secret in `config/aeroacars-integrity-gate.php` setzen.
7. **WICHTIG (Spec R8):** GSG-seitig auf jedem Rank `auto_approve_acars = FALSE` setzen — sonst greift PENDING-Hold nicht. Siehe LE30 Setup-Guide.
8. Migration ausführen: `php artisan migrate --path=Modules/AeroACARSIntegrityGate/database/migrations`.
9. Module aktivieren: `php artisan module:enable AeroACARSIntegrityGate`.
10. Tests: `vendor/bin/phpunit Modules/AeroACARSIntegrityGate/tests`.

---

## Sicherheits-Constraints (aus Spec übernommen)

- **HMAC-SHA256** (Module ↔ VPS) mit `timestamp + nonce + TTL=300s + replay-cache` (LE31a)
- **Shared-Secret** als ENV-Variable, nie in DB
- **Fail-Closed** auf PENDING-Hold bei JEDEM Failure (VPS-Timeout, 404, Throwable, Config-Drift) — LE33
- **Auto-Repair-Pattern**: boot + daily-cron + per-PIREP (LE30 G.2.0)
- **REJECTED-Emergency-Fallback** wenn Auto-Repair selbst exception wirft — LE30 R13
- **Listener vs Webhook Lifecycle-Split** — Listener nutzt direkte Eloquent-State-Manipulation (in-flow), Webhook nutzt `PirepService::accept()/reject()` (off-flow, lifecycle-events). LE33.

---

## Nicht gemacht in Slice 7 Scaffolding

Dieses Verzeichnis stellt **Skelett-Files mit TODO-Markern** bereit, nicht die fertigen Implementations. Die vollständige Spec-Implementation gehört in den dedizierten Repo, dort:

- Full Laravel-Test-Suite mit `RefreshDatabase` + phpVMS-Test-Helpers
- CI-Pipeline (GitHub Actions: phpunit + larastan)
- Composer-Package-Release-Workflow
- Module-Settings-Admin-UI (Blade)
- DE/EN-Übersetzungen
- README mit Setup-Guide für VAs (auto_approve_acars=false-Hinweis prominent)

**Aufwand-Schätzung Slice 7 in dediziertem Repo:** ~40h (Spec R6 Stream G).

---

## Wie die Code-Stubs zu lesen sind

Jedes Stub-File enthält:
1. PHPDoc-Header mit Spec-LE-Referenz
2. Klassen-Stub mit signatur-korrekten Methoden
3. `// TODO(Slice 7 PR-N): siehe Spec-Sektion X` Marker an jeder Implementierungs-Stelle
4. Inline-Doc-Block mit den Verhaltens-Anforderungen aus der Spec

So kann ein Implementor (oder Folge-Agent) die Stubs Punkt für Punkt mit der Spec-Vorlage befüllen.
