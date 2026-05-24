<?php
/**
 * pirep_review_metadata — pro-PIREP Integrity-Verdict + Review-Decision-Tracking
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  G.3
 *
 * Speichert die Module-↔-VPS-Konversation pro PIREP:
 *   - verdict_received_at / verdict_payload (VPS-Response)
 *   - integrity_state (held_review / held_untrusted / held_timeout /
 *                       held_no_telemetry / held_error / held_reopened /
 *                       clean / decided)
 *   - reviewer / reviewed_at / decision / reason (vom Webhook)
 */

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;

return new class extends Migration {
    public function up(): void
    {
        Schema::create('pirep_review_metadata', function (Blueprint $table) {
            $table->id();
            $table->string('pirep_id')->unique()->index();
            $table->string('integrity_state')->default('pending')->index();
            // 'pending' | 'clean' | 'held_review' | 'held_untrusted' |
            // 'held_timeout' | 'held_no_telemetry' | 'held_error' |
            // 'held_reopened' | 'held_emergency_repair_failed' | 'decided'

            $table->string('score_trust_level')->nullable();
            $table->text('score_trust_reasons')->nullable();  // JSON
            $table->text('integrity_flags')->nullable();       // JSON
            $table->string('recorder_version')->nullable();
            $table->timestamp('verdict_received_at')->nullable();

            $table->string('reviewer')->nullable();
            $table->timestamp('reviewed_at')->nullable();
            $table->string('decision')->nullable();  // 'accepted_with_warning' | 'rejected' | 'dismissed_fp' | 'reopen'
            $table->text('reason')->nullable();
            $table->string('decision_id')->nullable()->unique();

            $table->timestamps();
        });
    }

    public function down(): void
    {
        Schema::dropIfExists('pirep_review_metadata');
    }
};
