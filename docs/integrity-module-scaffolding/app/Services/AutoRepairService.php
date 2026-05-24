<?php
/**
 * AutoRepairService — auto_approve_acars-Config-Drift-Schutz
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md
 *       LE30 G.2.0 — boot + daily-cron + per-PIREP Defense-in-Depth
 *
 * WICHTIG (R8): Das gesamte Module hängt davon ab, dass auf JEDEM Rank
 * `auto_approve_acars = FALSE` gesetzt ist — sonst greift PENDING-Hold
 * nicht (phpVMS' submit-Code würde dann nach Listener noch
 * auto-approven). Dieser Service patcht stillschweigend:
 *   1. beim Boot (`checkAndRepairAllRanks`)
 *   2. täglich per Cron (`IntegrityAutoRepairCheck`)
 *   3. per-PIREP unmittelbar vor State-Manipulation (`repairForPirep`)
 *
 * Per-PIREP-Variante muss zusätzlich den in-memory-Rank refreshen
 * (`$pirep->load('user.rank')`), damit der Listener nicht auf einem
 * stale Wert aus der Same-Request-Relation arbeitet (LE30 R10 Bug).
 *
 * Bei JEDEM Drift-Find → Discord-Alarm + Log-Warning.
 */

namespace Modules\AeroACARSIntegrityGate\Services;

use App\Models\Pirep;
use App\Models\Rank;
use Illuminate\Support\Facades\Cache;
use Illuminate\Support\Facades\Log;

class AutoRepairService
{
    /** Boot/Cron-Variante: alle Ranks scannen + patchen. Idempotent. */
    public function checkAndRepairAllRanks(): int
    {
        // TODO(Slice 7 PR-1): Spec LE30 G.2.0 Code-Block
        // SELECT id, name, auto_approve_acars FROM ranks
        //   WHERE auto_approve_acars = 1
        // → UPDATE SET auto_approve_acars = 0
        // → Discord-Alarm
        // → return count patched
        $count = Rank::where('auto_approve_acars', true)->count();
        if ($count > 0) {
            Rank::where('auto_approve_acars', true)
                ->update(['auto_approve_acars' => false]);
            Cache::tags(['ranks'])->flush();
            Log::warning("AeroACARS-Gate auto-repair patched {$count} ranks");
            // TODO(Slice 7 PR-1): Discord-Alarm
        }
        return $count;
    }

    /** Per-PIREP-Variante: für den spezifischen Pilot-Rank prüfen + patchen + refresh. */
    public function repairForPirep(Pirep $pirep): void
    {
        // TODO(Slice 7 PR-1): Spec LE30 R11 Code
        $rank = $pirep->user->rank;
        if ($rank && $rank->auto_approve_acars) {
            $rank->auto_approve_acars = false;
            $rank->save();
            Cache::tags(['ranks'])->flush();
            $pirep->load('user.rank');  // refresh in-memory
            Log::warning('AeroACARS-Gate per-PIREP auto-repair', [
                'pirep_id' => $pirep->id, 'rank_id' => $rank->id,
            ]);
            // TODO(Slice 7 PR-1): Discord-Alarm
        }
    }
}
