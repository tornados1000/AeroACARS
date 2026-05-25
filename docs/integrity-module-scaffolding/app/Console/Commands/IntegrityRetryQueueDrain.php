<?php
/**
 * IntegrityRetryQueueDrain — Async Worker für VPS-Verdict-Retries
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE33
 *
 * Schedule: alle 5 Minuten
 *
 *   $schedule->command('aeroacars:integrity-retry-queue-drain')
 *            ->everyFiveMinutes()
 *            ->withoutOverlapping();
 *
 * Liest aus module_webhook_retry_queue rows mit next_attempt_at <= now()
 * und versucht erneut den VPS-Verdict zu holen + apply.
 *
 * TODO(Slice 7 PR-3): siehe LE33 Async-Retry Worker Code-Block.
 */

namespace Modules\AeroACARSIntegrityGate\Console\Commands;

use Illuminate\Console\Command;
use Modules\AeroACARSIntegrityGate\Services\AsyncVerdictRetryQueue;

class IntegrityRetryQueueDrain extends Command
{
    protected $signature = 'aeroacars:integrity-retry-queue-drain';
    protected $description = 'Drain pending VPS-verdict retries from module_webhook_retry_queue';

    public function handle(AsyncVerdictRetryQueue $queue): int
    {
        $drained = $queue->drain();
        $this->info("Drained {$drained} retry-queue entries");
        return self::SUCCESS;
    }
}
