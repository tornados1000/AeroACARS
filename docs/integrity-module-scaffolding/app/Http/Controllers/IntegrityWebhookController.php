<?php
/**
 * IntegrityWebhookController — Webhook für Admin-Entscheidungen vom VPS
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE34
 *
 * Webhook-Pfad: POST /api/aeroacars-gate/webhook/decision
 *
 * Vom VPS gepushed wenn ein VA-Admin im Review-Queue-Tab eine
 * Entscheidung getroffen hat:
 *   - accepted_with_warning → PirepService::accept() (live alle Hooks)
 *   - rejected              → PirepService::reject() (live alle Hooks)
 *   - dismissed_fp          → PirepService::accept() (state ACCEPTED,
 *                                 reasons-Spalte „dismissed false positive")
 *   - reopen                → nur von REJECTED → PENDING-held setzen
 *                                 (R14: NICHT von dismissed_fp/accepted_with_warning)
 *
 * Auth: HMAC-Header (LE31a). reason REQUIRED für rejected/dismissed_fp/reopen.
 *
 * Idempotenz: jede Decision hat ein decision_id im Payload — speichern in
 * module_webhook_retry_queue.processed_decisions, dedupliziert Re-Plays.
 */

namespace Modules\AeroACARSIntegrityGate\Http\Controllers;

use App\Http\Controllers\Controller;
use App\Services\PirepService;
use App\Models\Pirep;
use App\Models\Enums\PirepState;
use Illuminate\Http\Request;
use Illuminate\Http\JsonResponse;
use Modules\AeroACARSIntegrityGate\Services\HmacSigner;

class IntegrityWebhookController extends Controller
{
    public function __construct(
        private PirepService $pirepService,
        private HmacSigner $signer,
    ) {}

    public function decision(Request $request): JsonResponse
    {
        // ─── HMAC verify ─────────────────────────────────────────────
        $body = $request->getContent();
        if (!$this->signer->verify(
            method: 'POST',
            path: '/api/aeroacars-gate/webhook/decision',
            body: $body,
            hmac: $request->header('X-AeroACARS-HMAC', ''),
            tsMs: (int) $request->header('X-AeroACARS-TS', '0'),
            nonce: $request->header('X-AeroACARS-Nonce', ''),
        )) {
            return response()->json(['error' => 'hmac_invalid'], 401);
        }

        // ─── Validation ──────────────────────────────────────────────
        $data = $request->validate([
            'pirep_id' => 'required|integer|exists:pireps,id',
            'target_state' => 'required|in:accepted_with_warning,rejected,dismissed_fp,reopen',
            'reason' => 'required_if:target_state,rejected,dismissed_fp,reopen|string|nullable',
            'decision_id' => 'required|string',
            'reviewer' => 'required|string',
        ]);

        $pirep = Pirep::findOrFail($data['pirep_id']);

        // ─── reopen: nur von REJECTED (R14) ──────────────────────────
        if ($data['target_state'] === 'reopen' && $pirep->state !== PirepState::REJECTED) {
            return response()->json([
                'error' => 'reopen_only_allowed_from_rejected',
                'current_state' => $pirep->state,
            ], 400);
        }

        // ─── Apply via PirepService (live Lifecycle-Events) ──────────
        // TODO(Slice 7 PR-4): Spec LE34 Code-Block
        switch ($data['target_state']) {
            case 'accepted_with_warning':
            case 'dismissed_fp':
                $this->pirepService->accept($pirep);
                break;
            case 'rejected':
                $this->pirepService->reject($pirep);
                break;
            case 'reopen':
                // REJECTED → PENDING-held (state=PENDING, integrity_state='held_reopened')
                $pirep->state = PirepState::PENDING;
                $pirep->save();
                break;
        }

        // TODO(Slice 7 PR-4): pirep_review_metadata aktualisieren mit
        // reviewer + reason + decision_id + ts.

        return response()->json([
            'ok' => true,
            'pirep_id' => $pirep->id,
            'new_state' => $pirep->state,
            'decision_id' => $data['decision_id'],
        ]);
    }
}
