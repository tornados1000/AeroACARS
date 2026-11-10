<?php
/**
 * PirepFiledGate — Synchroner Listener für PirepFiled (LE30 R13)
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE30
 *
 * Ablauf (alle Schritte normativ aus der Spec):
 *   1. Auto-Repair-Defense-in-Depth (try/catch — bei Failure REJECTED-Emergency-Hold)
 *   2. Skip wenn schon final-state (defensive)
 *   3. VPS-Verdict einholen, State über PirepStateManipulator setzen
 *      - VpsUnreachable      → PENDING-Hold + AsyncRetry
 *      - VpsNoTelemetry (404)→ PENDING-Hold (fail-closed für v0.13.0+ Clients)
 *      - Any Throwable       → PENDING-Hold (fail-closed)
 *
 * SYNCHRONITÄT: dieser Listener IST NICHT `ShouldQueue` — phpVMS' submit-Flow
 * läuft synchron, wir müssen vor der Auto-Approve-Logik manipulieren.
 *
 * SYNC-TIMEOUT: max 3,5s (LE30) — 3s VpsClient-HTTP + 0,5s Buffer.
 */

namespace Modules\AeroACARSIntegrityGate\Listeners;

use App\Events\PirepFiled;
use App\Models\Enums\PirepState;
use Illuminate\Support\Facades\Log;
use Modules\AeroACARSIntegrityGate\Exceptions\AutoRepairVerificationFailedException;
use Modules\AeroACARSIntegrityGate\Exceptions\VpsNoTelemetryException;
use Modules\AeroACARSIntegrityGate\Exceptions\VpsUnreachableException;
use Modules\AeroACARSIntegrityGate\Services\AsyncVerdictRetryQueue;
use Modules\AeroACARSIntegrityGate\Services\AutoRepairService;
use Modules\AeroACARSIntegrityGate\Services\PirepStateManipulator;
use Modules\AeroACARSIntegrityGate\Services\VpsClient;

class PirepFiledGate
{
    public function __construct(
        private VpsClient $vpsClient,
        private AutoRepairService $autoRepairService,
        private PirepStateManipulator $pirepStateManipulator,
        private AsyncVerdictRetryQueue $asyncRetryQueue,
    ) {}

    public function handle(PirepFiled $event): void
    {
        $pirep = $event->pirep;

        // ─── SCHRITT 1: Auto-Repair-Defense-in-Depth ─────────────────────
        // TODO(Slice 7 PR-1): siehe LE30 Spec, exakter Code-Block
        try {
            $this->autoRepairService->repairForPirep($pirep);
            $pirep->load('user.rank');

            if ($pirep->user->rank->auto_approve_acars) {
                throw new AutoRepairVerificationFailedException(sprintf(
                    'Rank %d (%s) still has auto_approve_acars=true after repair attempt',
                    $pirep->user->rank->id, $pirep->user->rank->name
                ));
            }
        } catch (\Throwable $repairException) {
            Log::critical('AeroACARS-Gate: AUTO-REPAIR FAILED — emergency-hold', [
                'pirep_id' => $pirep->id,
                'rank_id' => $pirep->user->rank->id ?? null,
                'exception_class' => get_class($repairException),
                'exception_message' => $repairException->getMessage(),
            ]);
            $this->pirepStateManipulator->applyAutoRepairFailureFallback($pirep);
            // TODO(Slice 7 PR-1): Discord-Alarm
            return;
        }

        // ─── SCHRITT 2: Skip wenn schon final-state ─────────────────────
        if (in_array($pirep->state, [
            PirepState::ACCEPTED, PirepState::REJECTED,
            PirepState::CANCELLED, PirepState::DELETED,
        ], true)) {
            Log::info('AeroACARS-Gate: skipping PIREP in final state', [
                'pirep_id' => $pirep->id, 'state' => $pirep->state,
            ]);
            return;
        }

        // ─── SCHRITT 3: VPS-Verdict + State-Anwendung ───────────────────
        try {
            $verdict = $this->vpsClient->getIntegrityVerdict($pirep->id);
            $this->pirepStateManipulator->applyVerdict($pirep, $verdict);
        } catch (VpsUnreachableException $e) {
            Log::warning('AeroACARS-Gate: VPS unreachable, PENDING-Hold', [
                'pirep_id' => $pirep->id, 'error' => $e->getMessage(),
            ]);
            $this->pirepStateManipulator->applyTimeoutFallback($pirep);
            $this->asyncRetryQueue->enqueue($pirep->id);
        } catch (VpsNoTelemetryException $e) {
            Log::warning('AeroACARS-Gate: VPS has no telemetry, PENDING-Hold', [
                'pirep_id' => $pirep->id, 'user_id' => $pirep->user_id,
            ]);
            $this->pirepStateManipulator->applyNoTelemetryFallback($pirep);
        } catch (\Throwable $t) {
            Log::error('AeroACARS-Gate: internal error, PENDING-Hold (fail-closed)', [
                'pirep_id' => $pirep->id,
                'throwable_class' => get_class($t),
                'message' => $t->getMessage(),
            ]);
            $this->pirepStateManipulator->applyInternalErrorFallback($pirep, $t);
        }
    }
}
