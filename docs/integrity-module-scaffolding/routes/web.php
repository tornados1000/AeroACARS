<?php

use Illuminate\Support\Facades\Route;
use Modules\AeroACARSIntegrityGate\Http\Controllers\IntegrityWebhookController;

// VPS-→-Module Webhook für Admin-Entscheidungen aus dem Review-Tab
// Auth: HMAC-Header in Controller (LE31a)
Route::post('/api/aeroacars-gate/webhook/decision',
    [IntegrityWebhookController::class, 'decision'])
    ->name('aeroacars-gate.webhook.decision');

// TODO(Slice 7 PR-4): Admin-Settings-Page (Blade)
// Route::get('/admin/aeroacars-integrity-gate', [SettingsController::class, 'index'])
//     ->middleware(['auth', 'admin'])
//     ->name('aeroacars-gate.admin.index');
