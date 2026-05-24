<?php
/**
 * Unit-Test für HmacSigner — sign/verify Roundtrip + TTL + Tamper-Detect.
 *
 * Spec: docs/spec/v0.13.0-mid-session-integrity-and-resume-policy.md  LE31a
 *
 * Test-Matrix:
 *   T1.1 sign() liefert hmac/ts/nonce
 *   T1.2 verify() akzeptiert direkt nach sign()
 *   T1.3 verify() lehnt ab bei verändertem body
 *   T1.4 verify() lehnt ab bei verändertem path
 *   T1.5 verify() lehnt ab nach TTL (301s alt)
 *   T1.6 verify() lehnt ab bei wrong secret
 *
 * TODO(Slice 7 PR-2): Implementierung dieser Tests im neuen Repo
 * mit phpunit/phpunit + orchestra/testbench.
 */

namespace Modules\AeroACARSIntegrityGate\Tests\Unit;

use PHPUnit\Framework\TestCase;
use Modules\AeroACARSIntegrityGate\Services\HmacSigner;

class HmacSignerTest extends TestCase
{
    public function test_sign_returns_hmac_ts_nonce(): void
    {
        // TODO(Slice 7 PR-2)
        $this->markTestIncomplete('To be implemented after composer install in new repo');
    }
}
