<?php
/**
 * PirepStateManipulator — direkte Eloquent-State-Manipulation (in-flow)
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE33
 *
 * WICHTIG: dieses Service NUTZT NICHT `PirepService::accept()/reject()`!
 * Begründung Lifecycle-Boundary (LE33 R9):
 *  - Listener läuft IN-FLOW von phpVMS' submit() — phpVMS feuert dort
 *    selbst `accept()/reject()` einmal auf den final-state. Wenn wir
 *    `PirepService::accept()` aufrufen würden, hätten wir Double-Trigger
 *    von Acceptance-Events (Discord-Posts, Mailer, Stats-Recompute,
 *    XP-Award, ...). Stattdessen direkter Eloquent-Save mit
 *    `$pirep->state = ...; $pirep->save()` und phpVMS macht den
 *    Lifecycle-Trigger via submit() für uns.
 *
 *  - Webhook (LE34, off-flow) NUTZT `PirepService::accept()/reject()`,
 *    weil es dort kein phpVMS-submit() gibt, das die Lifecycle-Events
 *    feuern würde.
 */

namespace Modules\AeroACARSIntegrityGate\Services;

use App\Models\Pirep;
use App\Models\Enums\PirepState;

class PirepStateManipulator
{
    /**
     * Spec LE33: Anwendung des VPS-Verdicts auf den PIREP. Setzt
     * Status + integrity_state + integrity_metadata + comment.
     *
     * @param array{verdict: string, score_trust_level: string, reasons: array, integrity_flags: array, recorder_version: string} $verdict
     */
    public function applyVerdict(Pirep $pirep, array $verdict): void
    {
        // TODO(Slice 7 PR-3): Spec LE33 Code-Block
        // - verdict == 'clean'      → state = ACCEPTED (oder lass auf PENDING wenn
        //                              dieser Pilot/Rank reqd review)
        // - verdict == 'review'     → state = PENDING + integrity_state = 'held_review'
        // - verdict == 'untrusted'  → state = PENDING + integrity_state = 'held_untrusted'
        // - verdict == 'no_telemetry' → applyNoTelemetryFallback
        //
        // Plus: pirep_review_metadata row mit score_trust_level + reasons
        // schreiben (DB-Tabelle aus Migration).
    }

    public function applyTimeoutFallback(Pirep $pirep): void
    {
        // TODO(Slice 7 PR-3): PENDING-Hold mit integrity_state = 'held_timeout'
    }

    public function applyNoTelemetryFallback(Pirep $pirep): void
    {
        // TODO(Slice 7 PR-3): PENDING-Hold mit integrity_state = 'held_no_telemetry'
    }

    public function applyInternalErrorFallback(Pirep $pirep, \Throwable $error): void
    {
        // TODO(Slice 7 PR-3): PENDING-Hold mit integrity_state = 'held_error'
        // + error_message in metadata
    }

    public function applyAutoRepairFailureFallback(Pirep $pirep): void
    {
        // Spec LE30 R13: jeder Auto-Repair-Failure → REJECTED-Emergency-Hold.
        // Das ist der EINZIGE Pfad in diesem Module der REJECTED nutzt —
        // weil bei Auto-Repair-Failure haben wir keine Garantie dass
        // PENDING wirklich hält (auto_approve_acars könnte noch true sein).
        // REJECTED ist der einzige State der submit() überlebt.
        $pirep->state = PirepState::REJECTED;
        $pirep->status = 'INT_REJ';  // 'Integrity gate rejected, emergency-hold'
        $pirep->save();
        // TODO(Slice 7 PR-3): integrity_metadata schreiben
    }
}
