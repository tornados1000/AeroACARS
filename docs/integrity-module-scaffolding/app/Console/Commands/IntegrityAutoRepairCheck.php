<?php
/**
 * IntegrityAutoRepairCheck — Daily Cron für auto_approve_acars-Drift
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE30 G.2.0
 *
 * Schedule: täglich um 04:00 UTC (in App\Console\Kernel::schedule eintragen)
 *
 *   $schedule->command('aeroacars:integrity-auto-repair-check')
 *            ->dailyAt('04:00')
 *            ->withoutOverlapping()
 *            ->runInBackground();
 */

namespace Modules\AeroACARSIntegrityGate\Console\Commands;

use Illuminate\Console\Command;
use Modules\AeroACARSIntegrityGate\Services\AutoRepairService;

class IntegrityAutoRepairCheck extends Command
{
    protected $signature = 'aeroacars:integrity-auto-repair-check';
    protected $description = 'Daily scan of ranks.auto_approve_acars — patches any rank with =true to =false (AeroACARS-Gate Defense-in-Depth)';

    public function handle(AutoRepairService $svc): int
    {
        $count = $svc->checkAndRepairAllRanks();
        if ($count > 0) {
            $this->warn("Auto-repaired {$count} rank(s) where auto_approve_acars had drifted to true");
        } else {
            $this->info("All ranks clean.");
        }
        return self::SUCCESS;
    }
}
