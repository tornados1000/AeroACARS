<?php
/**
 * VpsClient — HTTP-Client für den AeroACARS-Live-Recorder
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md
 *       LE31 (single-attempt sync + async retry) + LE31a (HMAC-Auth)
 *
 * Verantwortung:
 *  - getIntegrityVerdict($pirepId): mache GET /api/integrity-check/<pirep_id>
 *    mit HMAC-Header, parse JSON-Response zum Verdict.
 *  - Timeout: 3s (Sync-Budget aus LE30 = 3,5s gesamt).
 *  - KEIN In-Sync-Retry mehr (R6 hatte 3 Attempts × 5s, war zu lang).
 *  - 404 → VpsNoTelemetryException
 *  - Timeout/Connect-Fail → VpsUnreachableException
 *  - 5xx → VpsUnreachableException (async retry kann sich kümmern)
 *  - 2xx → VerdictDto
 */

namespace Modules\AeroACARSIntegrityGate\Services;

use GuzzleHttp\Client;
use GuzzleHttp\Exception\ConnectException;
use GuzzleHttp\Exception\RequestException;
use Modules\AeroACARSIntegrityGate\Exceptions\VpsNoTelemetryException;
use Modules\AeroACARSIntegrityGate\Exceptions\VpsUnreachableException;

class VpsClient
{
    private const TIMEOUT_SECONDS = 3.0;

    public function __construct(
        private HmacSigner $signer,
    ) {}

    /**
     * @return array{
     *   verdict: 'clean' | 'review' | 'untrusted' | 'no_telemetry',
     *   score_trust_level: string,
     *   reasons: array<int, array{code: string, detail: array<string, mixed>}>,
     *   integrity_flags: array<int, mixed>,
     *   recorder_version: string,
     * }
     */
    public function getIntegrityVerdict(int $pirepId): array
    {
        // TODO(Slice 7 PR-2): siehe LE31 Spec, exakter Code-Block
        // - URL aus config('aeroacars-integrity-gate.vps_url')
        // - Pfad: /api/integrity-check/{pirep_id}
        // - Headers: X-AeroACARS-HMAC, X-AeroACARS-TS, X-AeroACARS-Nonce
        // - HTTP-200 → json_decode → return array
        // - HTTP-404 → throw VpsNoTelemetryException
        // - Timeout/Connect-Fail → throw VpsUnreachableException
        throw new \RuntimeException('not yet implemented — see LE31');
    }
}
