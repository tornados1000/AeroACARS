<?php
/**
 * AsyncVerdictRetryQueue — Persistente Async-Queue für VPS-Retry
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE33
 *
 * - enqueue(pirepId): row in module_webhook_retry_queue mit
 *   next_attempt_at = now+60s.
 * - drain(): liest pending rows, ruft VpsClient::getIntegrityVerdict,
 *   apply via PirepStateManipulator, exponential backoff bei Failure.
 *
 * Max-Attempts = 10, dann „give up" (PIREP bleibt in PENDING-Hold —
 * Admin muss manuell entscheiden über Webhook bzw. phpVMS-Panel).
 *
 * TODO(Slice 7 PR-3): siehe LE33 Spec Code-Block.
 */

namespace Modules\AeroACARSIntegrityGate\Services;

use App\Models\Pirep;
use Illuminate\Support\Facades\DB;

class AsyncVerdictRetryQueue
{
    public function enqueue(int $pirepId): void
    {
        // TODO(Slice 7 PR-3): INSERT into module_webhook_retry_queue
        DB::table('module_webhook_retry_queue')->insert([
            'pirep_id' => (string) $pirepId,
            'action' => 'fetch_verdict',
            'next_attempt_at' => now()->addSeconds(60),
            'created_at' => now(),
            'updated_at' => now(),
        ]);
    }

    public function drain(): int
    {
        // TODO(Slice 7 PR-3): SELECT pending rows, retry, backoff
        return 0;
    }
}
