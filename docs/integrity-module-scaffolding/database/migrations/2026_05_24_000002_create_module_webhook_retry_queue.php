<?php
/**
 * module_webhook_retry_queue — Async-Retry-Queue für VPS-Verdict-Abrufe
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md
 *       LE33 Async-Retry-Worker
 *
 * Wenn der synchrone Listener-Call zum VPS scheitert (Timeout, 5xx),
 * landet der pirep_id hier — ein Worker drained alle paar Minuten und
 * versucht erneut den Verdict zu holen, dann State-Update wie ursprünglich.
 *
 * target_state-Spalten-Kommentar (R13): inkludiert dismissed_fp.
 */

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;

return new class extends Migration {
    public function up(): void
    {
        Schema::create('module_webhook_retry_queue', function (Blueprint $table) {
            $table->id();
            $table->string('pirep_id')->index();
            $table->string('action')->default('fetch_verdict');
            // 'fetch_verdict' | 'apply_webhook_decision'
            $table->string('target_state')->nullable();
            // accepted_with_warning | rejected | dismissed_fp | reopen
            $table->text('payload_json')->nullable();
            $table->integer('attempts')->default(0);
            $table->timestamp('next_attempt_at')->nullable()->index();
            $table->timestamp('last_attempt_at')->nullable();
            $table->text('last_error')->nullable();
            $table->timestamp('succeeded_at')->nullable();
            $table->timestamps();
        });
    }

    public function down(): void
    {
        Schema::dropIfExists('module_webhook_retry_queue');
    }
};
