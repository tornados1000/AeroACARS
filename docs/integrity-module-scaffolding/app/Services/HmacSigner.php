<?php
/**
 * HmacSigner — HMAC-SHA256-Auth zwischen Module ↔ VPS
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md
 *       LE31a (R7 — ersetzt R6 plain shared-secret)
 *
 * Pattern:
 *   string_to_sign = "{method}\n{path}\n{timestamp_ms}\n{nonce}\n{body_sha256}"
 *   signature      = hmac_sha256(shared_secret, string_to_sign)
 *
 * Headers:
 *   X-AeroACARS-HMAC:   <hex signature>
 *   X-AeroACARS-TS:     <ms-timestamp>
 *   X-AeroACARS-Nonce:  <random 16 byte hex>
 *
 * Server-Seite (recorder) validiert:
 *   - timestamp within now ± TTL (300s)
 *   - nonce nicht im replay-cache der letzten 600s
 *   - signature mit konstanter Zeit verglichen (hash_equals)
 */

namespace Modules\AeroACARSIntegrityGate\Services;

class HmacSigner
{
    private string $secret;
    private const TTL_SECONDS = 300;

    public function __construct()
    {
        $this->secret = config('aeroacars-integrity-gate.shared_secret', '');
    }

    /** @return array{hmac: string, ts: int, nonce: string} */
    public function sign(string $method, string $path, string $body = ''): array
    {
        // TODO(Slice 7 PR-2): siehe LE31a Spec
        $tsMs = (int) (microtime(true) * 1000);
        $nonce = bin2hex(random_bytes(16));
        $bodyHash = hash('sha256', $body);
        $stringToSign = "{$method}\n{$path}\n{$tsMs}\n{$nonce}\n{$bodyHash}";
        $hmac = hash_hmac('sha256', $stringToSign, $this->secret);
        return ['hmac' => $hmac, 'ts' => $tsMs, 'nonce' => $nonce];
    }

    public function verify(string $method, string $path, string $body, string $hmac, int $tsMs, string $nonce): bool
    {
        // TODO(Slice 7 PR-2): TTL-Check + replay-cache + constant-time compare
        // Server-Seite spiegelt sign() — siehe Spec LE31a für komplette
        // Implementation inkl. replay-cache (Redis/Cache::add).
        $age = (int)(microtime(true) * 1000) - $tsMs;
        if ($age < -5000 || $age > self::TTL_SECONDS * 1000) return false;
        $bodyHash = hash('sha256', $body);
        $expected = hash_hmac('sha256', "{$method}\n{$path}\n{$tsMs}\n{$nonce}\n{$bodyHash}", $this->secret);
        return hash_equals($expected, $hmac);
    }
}
