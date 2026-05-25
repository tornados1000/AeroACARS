<?php
/**
 * AeroACARS Integrity Gate — phpVMS Module ServiceProvider
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md
 *       Stream G — phpVMS Truth-Sync via PirepFiled Listener
 *
 * Verantwortlichkeiten:
 *  - Event-Listener-Binding für PirepFiled → PirepFiledGate
 *  - Routes-Loading (Webhook für Admin-Entscheidungen, Settings-Page)
 *  - Config-Publishing (aeroacars-integrity-gate.php)
 *  - Migration-Loading
 *  - Console-Command-Binding (AutoRepairCheck daily, RetryQueueDrain)
 */

namespace Modules\AeroACARSIntegrityGate;

use Illuminate\Support\Facades\Event;
use Illuminate\Support\ServiceProvider;
use App\Events\PirepFiled;
use Modules\AeroACARSIntegrityGate\Listeners\PirepFiledGate;
use Modules\AeroACARSIntegrityGate\Services\AutoRepairService;
use Modules\AeroACARSIntegrityGate\Services\VpsClient;
use Modules\AeroACARSIntegrityGate\Services\HmacSigner;
use Modules\AeroACARSIntegrityGate\Services\PirepStateManipulator;
use Modules\AeroACARSIntegrityGate\Services\AsyncVerdictRetryQueue;

class AeroACARSIntegrityGateServiceProvider extends ServiceProvider
{
    public function boot(): void
    {
        $this->loadRoutesFrom(__DIR__ . '/../routes/web.php');
        $this->loadMigrationsFrom(__DIR__ . '/../database/migrations');
        $this->loadViewsFrom(__DIR__ . '/../resources/views', 'aeroacars-integrity-gate');
        $this->loadTranslationsFrom(__DIR__ . '/../resources/lang', 'aeroacars-integrity-gate');

        $this->publishes([
            __DIR__ . '/../config/aeroacars-integrity-gate.php' => config_path('aeroacars-integrity-gate.php'),
        ], 'aeroacars-integrity-gate-config');

        // Spec LE30: synchroner Listener — KEIN ShouldQueue, sonst greift
        // die State-Manipulation nicht vor phpVMS' Auto-Approve.
        Event::listen(PirepFiled::class, PirepFiledGate::class);

        // ──────────────────────────────────────────────────────────
        // Spec LE30 G.2.0 — Boot-Time-Auto-Repair-Check
        // ──────────────────────────────────────────────────────────
        // Beim App-Boot einmal die Config-Drift-Prüfung laufen lassen.
        // Schützt vor dem Fall dass jemand zwischen Releases auto_approve_acars
        // wieder auf true gesetzt hat — Auto-Repair patcht das stillschweigend
        // mit Discord-Alert. Idempotent.
        try {
            $this->app->make(AutoRepairService::class)->checkAndRepairAllRanks();
        } catch (\Throwable $e) {
            \Log::warning('AeroACARS-Gate: boot-time auto-repair failed', [
                'message' => $e->getMessage(),
            ]);
        }
    }

    public function register(): void
    {
        $this->mergeConfigFrom(
            __DIR__ . '/../config/aeroacars-integrity-gate.php',
            'aeroacars-integrity-gate'
        );

        // ──────────────────────────────────────────────────────────
        // Console-Commands für Daily-Cron + Async-Retry
        // ──────────────────────────────────────────────────────────
        if ($this->app->runningInConsole()) {
            $this->commands([
                Console\Commands\IntegrityAutoRepairCheck::class,
                Console\Commands\IntegrityRetryQueueDrain::class,
            ]);
        }

        // ──────────────────────────────────────────────────────────
        // Service-Bindings (für Listener-Constructor-Injection)
        // ──────────────────────────────────────────────────────────
        $this->app->singleton(HmacSigner::class);
        $this->app->singleton(VpsClient::class);
        $this->app->singleton(AutoRepairService::class);
        $this->app->singleton(PirepStateManipulator::class);
        $this->app->singleton(AsyncVerdictRetryQueue::class);
    }
}
