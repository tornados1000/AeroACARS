<?php
/**
 * AeroACARS Integrity Gate — Module-Config
 *
 * Werte werden aus ENV-Variablen gelesen. Niemals shared_secret in DB
 * oder in committed Files speichern.
 */

return [
    /*
    |--------------------------------------------------------------------------
    | VPS Recorder URL
    |--------------------------------------------------------------------------
    | Base-URL des AeroACARS-Live-Recorders. Production:
    |   https://live.kant.ovh
    | Lokale Dev:
    |   http://localhost:8088
    */
    'vps_url' => env('AEROACARS_GATE_VPS_URL', 'https://live.kant.ovh'),

    /*
    |--------------------------------------------------------------------------
    | HMAC Shared Secret
    |--------------------------------------------------------------------------
    | Per Spec LE31a — wird bei jedem Module-↔-VPS-Call beide-seitig
    | mit dem string_to_sign HMAC-SHA256-gehasht. Muss auf Modul und
    | Recorder identisch sein. Mindestens 256 Bit Entropie:
    |   openssl rand -hex 32
    */
    'shared_secret' => env('AEROACARS_GATE_SHARED_SECRET', ''),

    /*
    |--------------------------------------------------------------------------
    | Sync-Timeout
    |--------------------------------------------------------------------------
    | Per Spec LE30 = max 3,5s gesamtem Listener-Pfad. VpsClient nutzt
    | 3s davon, 0.5s Buffer für State-Manipulation.
    */
    'vps_timeout_seconds' => env('AEROACARS_GATE_VPS_TIMEOUT', 3.0),

    /*
    |--------------------------------------------------------------------------
    | Discord Webhook für Alarms
    |--------------------------------------------------------------------------
    | Auto-Repair-Failures und Config-Drift werden hier gemeldet. Optional —
    | wenn leer wird nur in Laravel-Log geschrieben.
    */
    'discord_webhook_url' => env('AEROACARS_GATE_DISCORD_WEBHOOK', ''),

    /*
    |--------------------------------------------------------------------------
    | Auto-Repair Settings
    |--------------------------------------------------------------------------
    | Spec LE30 G.2.0 — Boot-Time-Auto-Repair-Check ein/aus. Default an.
    */
    'auto_repair_on_boot' => env('AEROACARS_GATE_AUTO_REPAIR_ON_BOOT', true),
];
