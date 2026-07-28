# Changelog

All notable changes to AeroACARS. Format: loosely based on [Keep a Changelog](https://keepachangelog.com/); version numbers follow [Semantic Versioning](https://semver.org/) (Patch: bugfix, Minor: feature, Major: breaking).

---

## [v0.18.5] — 2026-07-06 · Health-Report Follow-Up

Re-measurement 2026-07-06 (cross-session health-report review): the ATCCOM cases are resolved, but "FALCON 50" and "A400M" were completely missing from `map_model_name_to_icao()`, and suffix variants like "A350-900 ULR" fell through the exact literal list. Both added + prefix fallback for variant-rich families. Before release, a multi-agent code review (8 finder angles + verification) ran over this AND the analogous fix in the aeroacars-live audit tooling — in the process, two real gaps were found in the audit tool (not in this client diff) and fixed separately: a case-sensitivity bug + a missing junk-sentinel filter in `cleanAircraftIcao` (see aeroacars-live commit 29cf55b). Details: `docs/release-notes/v0.18.5.md`.

---

## [v0.18.4] — 2026-07-06 · Aircraft-Scan Operation Reworked

Final follow-up to the folder selection introduced in v0.18.2. Fix: a manually selected/typed path was previously only APPENDED to the full auto-detection instead of replacing it — "Search aircraft" still searched the entire library again, mixed in with the selected folder (field finding by Thomas K.). Root selection extracted into `select_roots()` and made testable: a manual path is now the ONLY search root. The UI was split accordingly: two fixed, always-visible buttons ("Search aircraft" for the full auto-detection, "Scan this folder" for exclusively the selected path) instead of one button that changes its behavior/text depending on the input field. Details: `docs/release-notes/v0.18.4.md`.

---

## [v0.18.3] — 2026-07-05 · Hardening LAN Remote Control

Thomas's hint (following up on the v0.18.2 finding about the double-log cause): could the duplicates be coming from the LAN remote control? Not the exact mechanism of the already-fixed bug, but the question uncovered an independent gap: `flight_resume_confirm` is reachable via `remote/bridge.rs`, but the frontend double-click protection (`confirmingRef`) only lives in the main-window React tree, not in the separate remote web interface. `spawn_position_streamer` and `spawn_phpvms_position_worker` already have a compare-and-swap guard against exactly this ("multiple flight_resume etc.") — `spawn_touchdown_sampler` did not. Guard added. Details: `docs/release-notes/v0.18.3.md`.

---

## [v0.18.2] — 2026-07-05 · Aircraft-Scan Follow-Up Fixes

Four fixes straight from pilot findings (Thomas K.): (1) ZIP64 sentinel bug — `.large_file(true)` during ZIP construction made the server (Node/fflate) falsely read EVERY file as ~4 GB, which broke the entire client upload path for EVERY submission since v0.18.0 (the web uploader was not affected); fix + byte-level regression test against the local file header. (2) X-Plane aircraft in category folders (e.g. "Aircraft/Extra Aircraft/…") were not found by the single-level search — now checked one level deeper if a level is not itself a package. (3) No protection against a second AeroACARS process (minimize-to-tray + restart after a sleep/wake cycle) → duplicate phase/touchdown rows in the PIREP; `tauri-plugin-single-instance` added. (4) The AGL/terrain line in the logbook altitude profile was hidden by the MSL line (drawing order, not a data issue) — fixed. New: native folder-selection dialog in the aircraft-scan panel. Details: `docs/release-notes/v0.18.2.md`.

---

## [v0.18.1] — 2026-07-05 · Aircraft Scan, Falcon 50 & Zibo 737 (X-Plane)

New aircraft-scan tool (Settings → Aircraft-Scan): submit a loaded aircraft add-on directly instead of uploading the entire Community folder, for faster premium-profile development. Two new profiles: Contrail Falcon 50 (FMA, autothrottle, master/fire warning) and Zibo/Laminar 737-800 (X-Plane, AP modes via `laminar/B738` DataRefs). Fix: standstill fallback for auto-file on the Falcon 50 (a sticking engine counter) — deliberately gated ONLY for aircraft with an error confirmed via aircraft-scan (`AircraftProfile::engine_count_unreliable`), 5-minute dwell, parking brake remains mandatory; no fleet-wide fallback, so that a taxi stop while waiting for taxi clearance at large airports doesn't get falsely counted as an arrival for any other aircraft (QA objection from Thomas, v0.18.0 discarded internally without publication). Fix: TakeoffRoll trailing edge after a rejected takeoff. Fix: hardened ICAO type derivation (fixed 42% garbage data in internal capture). New: GitHub Actions CI builds/tests on every push. Details: `docs/release-notes/v0.18.1.md`.

---

## [v0.12.4] — 2026-05-21 · Score Consistency Pilot App ↔ VPS Web

Spec: `docs/spec/v0.12.4-score-consistency.md` (QA R0→R10).

The same flight showed **different landing scores** in the VPS web under "Reports" and in "Landing Analysis". Cause: the webapp **recomputed** the sub-scores server-side from the raw touchdown fields instead of using the PIREP values supplied by the pilot client — and some of those raw fields were stale or incorrectly computed. As of v0.12.4, the pilot client is the **sole score authority**: it scores during the flight, the recorder propagates `landing_score` + `sub_scores` from the PIREP to the linked touchdown row, and the webapp now **only renders** — no more recompute in five views. This guarantees that the pilot app and VA web show the same score.

Alongside this, two raw touchdown field bugs were fixed: (1) `rollout_distance_m` was frozen as a mid-rollout snapshot ~9 s after touchdown in the `touchdown_complete` event — the client now sends a follow-up `touchdown_rollout_finalized` event with the final value after rollout finalization (~40 kt / heading turn-off); (2) `fuel_efficiency_pct` in the MQTT touchdown payload was incorrectly computed from block fuel (incl. taxi-out) instead of trip burn (`takeoff − landing`). The rollout finalization logic itself (40 kt / 30° turn-off / 5 kt floor) was already correct — only stale code comments were corrected along with it. Also new: the discrete landing category (Smooth/Acceptable/Firm/Hard/Severe) appears again in the webapp lists as a small word pill. For pilots, **nothing** changes about the scoring — the score algorithm is unchanged; v0.12.4 only establishes consistency between the pilot app and VA web.

## [v0.12.3] — 2026-05-21 · Landing-G FOQA-Compliant Measurement

Spec: `docs/spec/v0.12.3-landing-g-foqa-measurement.md` (SPEC ACCEPTED, QA R0→R5).

The touchdown G-force was previously scored as a **raw 50 Hz single-frame peak** — sharper than any real flight recorder would ever capture (finding TAP533: 1.95 g for an otherwise clean landing). As of v0.12.3, the G-force is measured like a real flight-data monitoring system (FOQA/FDM): a framerate-independent EMA filter (τ ≈ 100 ms) lightly smooths the G signal, then the peak value is taken over the touchdown window. This scored value (`scored_g`) consistently feeds **all** consumers — the `sub_g_force` score, touchdown classification, G-force forensics card, activity/ACARS text, the phpVMS PIREP field "Landing G-Force", QuickFlags, the RunwayDiagram, the `LandingScored` event, and the VPS webapp. The raw 50 Hz peak (`peak_g_load`) is kept everywhere as a forensic detail — backward-compatible, old PIREPs/JSONL logs deserialize unchanged. Accident/crash detection deliberately continues to use the raw extreme value (extreme-value detection, not fair scoring). The method is sim-agnostic — identical for MSFS and X-Plane. Alongside this: the G-force forensics card (pilot client + VPS webapp) was reworked in content (scored value as headline, raw values as forensic detail), and the `aeroacars-live` repo followed in parallel.

## [v0.12.2] — 2026-05-20 · X-Plane Aircraft DataRef Profiles (CL650) · Discord Push Diagnostics

Two streams. Spec: `docs/spec/v0.12.2-xplane-aircraft-dataref-profiles.md`.

**Stream 1 — X-Plane Aircraft DataRef Profiles (real solution for study-level add-ons).** Study-level X-Plane add-ons like the Hot-Start Challenger 650 drive cockpit/system functions via their own DataRefs and don't serve the standard `sim/...` DataRefs — AeroACARS never saw the CL650's flaps (GSG225 finding). v0.12.1 caught this fail-soft; v0.12.2 delivers the real solution. A profile system recognizes a known study-level aircraft via the web API aircraft title **or** an RREF probe on a signature DataRef, and then subscribes to its add-on-specific DataRefs. First profile: the Hot-Start Challenger 650 — flaps (lever 0/20/30/45, verified against the manufacturer documentation `Wires.txt`), battery master, beacon and taxi light. LANDING-CONFIG checking and approach-stability scoring now work fully for the CL650 again. On an in-flight aircraft change, the adapter automatically falls back to the standard catalog; aircraft without a profile remain unchanged. Code QA findings R4 (P1: reset without web API title) + R5 (P2: stale title didn't override a retired probe) incorporated.

**Stream 2 — Discord push diagnostics.** Pilots reported that Discord live presence disappeared after the updates while the test button kept working (= connection ok, but the flight push doesn't land). The push path silently swallowed every error message. The Discord panel (Settings → Discord) now shows "Last Discord push: X s ago" plus the last error message — if the time stands still during a flight, the push isn't getting through. Pure display/logging extension, no wire-format change.

## [v0.12.1] — 2026-05-20 · Pilot Findings · Phase FSM, VA Hardening, Approach Stability, Autostart, Resume

Five independent findings from one day's worth of pilot feedback, bundled into one release. Spec: `docs/spec/v0.12.1-phase-fsm-fix-and-va-hardening.md`.

**Stream A — Boarding→Pushback phase transition hardened against sim-reload glitches.** On flight BTX8815, the phase jumped to "Pushback" 65 s after flight start — triggered by a single telemetry glitch right after a sim reload (a brief speed spike even though the aircraft was standing still with the parking brake set). The transition now requires **real movement**: speed over two consecutive readings plus an actual position shift of at least 5 m. In addition, the recorder detects a reload gap (> 10 s without telemetry) and discards the first, glitch-prone sample afterward. Follow-on bug fixed: the off-block time (and thus the block time in the PIREP) is correct again.

**Stream B — VA hardening: only active GSG pilots.** Login, live tracking, and PIREP provisioning now check the pilot status from phpVMS. Only an **active** account gets through — pending, rejected, on-leave, suspended, or deleted accounts are rejected with a status-specific message. Enforcement runs both client- **and** server-side (recorder). Background: GitHub issue #16.

**Stream C — Approach stability fairer for study-level add-ons.** On flight GSG225 (Hot-Start Challenger 650, X-Plane), the approach-stability card showed "LANDING CONFIG: INCOMPLETE" even though gear and flaps were set — the study add-on doesn't serve the standard flaps DataRef. If the flaps position can't be reliably read, the landing config is now shown as "not assessable" instead of falsely red — and doesn't flow into the score as a penalty. Also: the "max V/S deviation < 500 ft" metric now ignores values below 50 ft AGL — a brief balloon during the flare is no longer an approach-stability issue.

**Stream D — Auto-start after update fixed (macOS).** After an app update, auto-start no longer worked until it was toggled off and back on once. Cause: a frontend timing bug overwrote the stored auto-start state at startup. Fixed — the stored state is now preserved.

**Stream E — Resume after crash made safer.** After a sim crash, the aircraft often reloads at an arbitrary ground position. Previously, resume automatically carried this glitch position into the running flight after a 10-second countdown. Now: if the sim position doesn't match the stored flight (flight was airborne, sim reports ground — or a large distance deviation), it is **not automatically** resumed — the pilot must actively confirm "Resume flight".

Webapp mirror (`aeroacars-live`) picks up the Stream B recorder hardening.

---

## [v0.12.0] — 2026-05-20 · Runway Utilization · Float Tolerance for Fair Braking Scoring

**Pilot complaint (BTX8815, Fenix A319, EDVE→LOWS):** excellent braking — short rollout distance (442 m) — and yet only 80 pts "runway utilization". On recalculation: the score was correct per the v0.10.0 formula, BUT the formula weighted the float distance (touching down too late) 1:1 like the rollout distance (the actual braking). On this flight, 55% of the "runway utilization" was float — the missing 20 pts came from the late touchdown, not from braking.

**Float tolerance (15% of LDA):** float within the first 15% of the usable runway length now counts as normal and costs no points — only the excess float above that flows into the point banding. The rollout distance still counts in full. This separates fair touchdown tolerance from real braking discipline in the score. The flight mentioned jumps from 80 to 100 pts.

**New rationale "long_float":** if the point loss came exclusively from the late touchdown (the rollout distance alone would have been excellent), the card now shows "Braking distance top — but touched down late" instead of a braking-critical rationale — with a coaching tip toward the aim point.

**UI:** the runway-utilization card now renders its info lines (touchdown point, rollout distance, runway) localized (DE/EN/IT). The "🛬 How is this calculated?" modal explains the float tolerance with a concrete example calculation. Terminology cleaned up: the rollout distance ends when clearing the runway (~40 kt), not at a full stop.

**Safety unchanged:** the overrun risk (too close to the runway end) is still checked against the real, un-shortened total distance — the float tolerance doesn't hide a real overrun.

**Score versioning:** `score_algorithm_version` 2 → 3. Forward-only — old PIREPs keep their stored score, nothing is recomputed retroactively. In the "My Flights" history, this can cause a small jump at the runway-utilization value between v2 and v3 flights; this is expected.

Spec: `docs/spec/v0.12.0-runway-utilization-refinement.md`. Webapp mirror (`aeroacars-live`) picks up the identical score logic.

---

## [v0.11.2] — 2026-05-18 · Pilot Wish · Touchdown Chart Also 3 s After TD

**Pilot wish (ViolonC on Discord):** the sink-rate chart in the landing detail shouldn't end hard at the TD point, but should also show the first 3 seconds after touchdown (strut compression + rebound + stabilization).

Fix: `build_landing_record` now, in addition to the pre-TD samples (from `snapshot_buffer`), also appends the first 3000 ms from `post_touchdown_buffer` to the `touchdown_profile` array. The backend has been sampling post-TD at 50 Hz since v0.5.39 — the data previously only existed as a local JSONL forensics file, and now it appears in the visible chart.

Chart code unchanged (`VsCurveChart` in `LandingPanel.tsx` dynamically renders the entire profile range — as soon as the array contains `t_ms > 0`, the x-axis is automatically extended).

---

## [v0.11.1] — 2026-05-18 · Hotfix · client_version in the Touchdown/PIREP Payload

**Hotfix for v0.11.0.** The UI render code for the pilot-client-version pill was correct, but the `client_version` field was never sent along in the touchdown payload or PIREP payload — it only existed in the `FlightMeta` of the connect message and therefore never ended up in the DB row. Result: an empty pill for all flights.

Fix: added `client_version: Some(env!("CARGO_PKG_VERSION"))` to `TouchdownPayload` and `PirepPayload` + set at the respective build sites. The pill is now correctly populated for all flights submitted **after** the v0.11.1 update.

Plus: the webapp now also shows the pill in the **reports overview** on every PIREP card (previously only in the detail modal when clicking a card) — VA owners can scan the pilots' version distribution directly from the list.

---

## [v0.11.0] — 2026-05-18 · Pilot Aids, Approach-Stability Card, Loadsheet Polish, Settings Tabs

**Public release** for all pilots. The stable updater picks this up automatically. Bundles three larger UI extensions, one important bug fix, and a new VPS diagnostic display into one release.

### New — Pilot aids for landing evaluation

- **🛬 Runway utilization — in-app explanation.** New "🛬 How is this calculated?" button at the bottom of the rollout sub-score card opens a modal with the formula, all five point bands (excellent_margin … overrun_risk) incl. point values + colors, heavy bonus (5 pp for wide-bodies), pre-displaced cap (max 55 pts on too-early touchdown), six skip reasons, and an explanation of what the card fields mean. The pilot can independently understand why their score is what it is — no more asking on Discord. (`client/src/components/RunwayUtilizationHelpModal.tsx`)
- **🛬 Glossary modal now multilingual (DE/EN/IT).** Previously the glossary in the runway diagram was hardcoded in German. Now fully via i18n — 18 terms (threshold, TDZ, aim point, TCH, DDS, glideslope, rollout, AIRAC, AGL/fpm/kt etc.) in all three languages. (`client/src/components/RunwayGlossaryModal.tsx`)

### New — Approach-Stability card (analogous to the aeroacars-live webapp)

- **7-tile approach stability instead of a narrow 2-value indicator.** In the LandingPanel, the previous `StabilityIndicator` (only σ-V/S + σ-bank) is replaced by a full-fledged card:
  - **STABLE-GATE pill** top right (✓ STABLE / ⚠ PARTIAL / ✗ UNSTABLE — thresholds: 0 violations = stable, ≥2 hard or ≥3 total = unstable, otherwise partial)
  - Sub-line with HAT/AGL filter + sample count
  - Data-source line (MSFS / X-Plane)
  - **7 tiles**: V/S jerk · bank σ (filtered) · IAS σ · sink rate · landing config · V/S vs. 3° ILS · max V/S deviation <500ft — each tile has a colored band (good/ok/bad/missing) based on the FAA AC 120-71B tolerances from the backend (`compute_approach_stability_v2`)
  - **Coaching banner** at the bottom, dynamic (clean / partial / unstable)
  - **ⓘ Help modal** "What do these values mean?" explains the stable-gate concept, the pill logic, and every individual metric with threshold bands (DE/EN/IT). (`client/src/components/ApproachStabilityCard.tsx`, `ApproachStabilityHelpModal.tsx`)
- **Backend storage extended (forward-only).** Three already-computed values (`approach_vs_deviation_fpm`, `approach_max_vs_deviation_below_500_fpm`, `approach_used_hat`) previously only existed in the MQTT payload — now also in the locally persisted `LandingRecord`. Old `landing_history.json` entries remain readable via `serde(default)`; their stability card shows a legacy notice instead of the tiles. (`client/src-tauri/crates/storage/src/lib.rs`, `client/src-tauri/src/lib.rs`)

### New — Loadsheet area modernized ("plan-fidelity" score)

- **Fuel and weight tables side by side** as a 2-column grid (auto-fit, stacks on narrow screens). Per row: the ACTUAL value large and prominent, plan value as a subtle sub-line below (`"Plan 13884 kg"`), Δ as a rounded pill with trend icon (`✓` / `≈` / `▲` / `▼`).
- **Loadsheet score footer became a hero card** with an SVG donut ring (mount animation, gradient, glow), the five plan-vs-actual pills (block fuel · TOW · LDW · ZFW) next to it.
- **Section naming cleaned up** — the parent section is now called **"Loadsheet"** (instead of "FUEL" nested twice), the sub-cards remain "⛽ FUEL" and "⚖️ WEIGHT", the footer card "📋 PLAN FIDELITY" (previously "loadsheet rating").
- **Plan/actual fuel bar** gets a real colored Δ pill instead of the previous mini-text.

### New — Settings sorted into 5 tabs (instead of one endless list)

The settings panel was completely reorganized into:
- **🎮 Simulator** — only the sim selection, prominent and isolated (default tab for first-time users)
- **🛫 Required** — SimBrief integration, flight-recording behavior (auto-file, auto-start, approach advisories)
- **🎨 Comfort** — language & display, behavior (minimize-to-tray), storage (flight logs), error reporting, Discord RPC
- **✈️ Plugins** — PMDG Premium Telemetry, X-Plane Premium plugin (previously hidden under debug mode, but these are not debug tools)
- **🛠 Technical** — debug mode + sim debug panel + phpVMS heartbeat + orphan-flights cleanup

The tab choice is remembered in localStorage. The UI gets hint text below the tab bar explaining what each tab covers. All labels in DE/EN/IT.

### Bug fix — VFR input mask disappears on resize

The VFR modal (ManualFlightModal) lost all typed-in values (block fuel, flight time, cruise level, route, ZFW etc.) if the pilot resized the window mid-input. Two defensive fixes:
- **Form state is persisted in localStorage** (key `aeroacars.vfr.<bid_id>`) — survives any React remount, resize, or brief app reload. Values are deleted after a successful flight start or an explicit cancel.
- **Backdrop click hardened** — now only closes the modal in the `aircraft` stage (before aircraft selection). In the `plan` stage (pilot is typing values), an accidental backdrop touch/click is ignored; only the explicit cancel button closes it. (`client/src/components/ManualFlightModal.tsx`)

### New — Pilot client version in the VPS PIREP report

- `client_version` from the MQTT payload (= the pilot client's `CARGO_PKG_VERSION`, already sent since v0.7.x) is now shown in the webapp PIREP detail header as a small pill — `📱 AeroACARS v0.11.0`. The VA owner sees at a glance with which client version a PIREP was produced — important when diagnosing anomalies. (`aeroacars-live/webapp/src/components/LandingAnalysis.tsx`)

### Tools / QA

- **New i18n audit script `scripts/i18n-audit.mjs`.** Three checks: (1) PARITY between DE/EN/IT, (2) all `t("…")` keys referenced in code exist in the EN master, (3) DEAD-key warning. Plural-aware (`_one`/`_other`), template-aware (`` `prefix.${...}` ``), string-literal-aware (`"foo.bar.baz"` as a constant), underscore-suffix-aware (`` `prefix_${...}` ``). Invocation: `node scripts/i18n-audit.mjs [--strict]`.
- **3 i18n bugs found + fixed:** `sim.xplane_phase2` was missing in all locales (pilot saw the raw key on the X-Plane panel); `settings.delete_all_logs_done` used a `defaultValue` hack instead of clean i18next-v4 plural resolution; `actions.language_it` existed only in IT (orphan).
- **12 genuine leftovers deleted:** `phase_timeline.*` (feature removed a while ago, locale keys were still there).

### Not in this release — coming in v0.12.0

- **Push-message system** (admin-to-pilot inbox with confirmation + targeting) — gets its own spec round, is scope-wise a standalone feature with recorder DB schema, webapp admin UI, and client polling + inbox component.
- **Loadsheet score algorithm polish** (e.g. penalty-threshold refinement) — the score itself is at v0.3.0 state; only the visualization was modernized.

---

---

## [v0.9.2] — 2026-05-18 · GlitchTip + Discord Rich Presence (Public Release)

**Identical in content to v0.9.1.** Versioned again because v0.9.1 itself still had a bug even after 6 QA rounds (F18, see round 7 below) and v0.9.0/v0.9.1 had already existed as drafts multiple times. Clean cut.

**Round 7 (F18) after the 7th QA pass:**
- **F18 (P1):** the Discord RPC toggle stayed on "Disabled" after an AeroACARS restart even though the checkbox in Settings was ON. Cause: `init()` passed the persisted settings to `Manager::new()` (= already set the internal state to `enabled=true`), after which `apply_settings(same)` matched the `(was_enabled=true, new=true)` no-op branch and never called `enable()` → pipe never opened → status stayed disabled even though the UI was on. Silent feature loss.
  - **Fix (primary):** start the manager with `DiscordPresenceSettings::default()` (= `enabled=false`), then push the persisted settings via `apply_settings()` afterward → triggers a clean `(false → true)` transition → `enable()` → the Discord pipe is actually opened.
  - **Fix (defense-in-depth, new in v0.9.2):** `apply_settings` now additionally checks `client_bound` and fires an `enable()` if `enabled=true && !client_bound` — regardless of which match arm would otherwise apply. This makes the function robust against all init races, transient connect failures, or future code paths that set settings without a clean state sync. Makes F18 non-reproducible even if someone later accidentally breaks the init logic again.

## [v0.9.1] — 2026-05-18 · INTERNAL (draft, not published due to F18)

7-round QA cycle. F18 was discovered before the publish click → escalated to v0.9.2.

## [v0.9.1-original] — 2026-05-18 · GlitchTip + Discord Rich Presence (initial public release after QA)

**Identical in content to the internal v0.9.0 that never went live.** Version number jump to 0.9.1 because an internal v0.9.0 build was briefly (~15 min, 0 downloads) visible as `releases/latest` — the number is thus burned, the fresh public release starts at 0.9.1.

### In addition to v0.9.0 content — QA hotfix findings F1-F11 cleaned up:

**Round 1 (F1-F8):**
- **F1:** the release workflow auto-published the draft immediately after the build → now an `if: false` gate, publish must be clicked manually in the UI
- **F2:** Discord RPC falsely showed "PREFLIGHT" for the phases `HOLDING` + `PIREP FILED` → all 20 phases now fully mapped + regression tests
- **F3:** Sentry opt-out called `flush()` (= actively pushed events out instead of discarding them) → removed
- **F4:** the tag `route` was in 4 allowlists but the UI said "route NOT sent" → consistently deleted
- **F5:** UI text said `live.kant.ovh` instead of the correct `tip.kant.ovh` (GDPR consent consistency)
- **F6:** frontend Sentry had `integrations: []` → disabled all default integrations (BrowserApiErrors, GlobalHandlers, Breadcrumbs); now active
- **F7:** `set_sim_lost` code exists but has no caller → wired up after all in round 5 (see F16)
- **F8:** CHANGELOG claim "18 phases" → corrected to 20 phases (17 FSM-active + 3 v0.10.0-ready)

**Round 2 (F9-F11) after 2nd QA pass:**
- **F9:** Sentry opt-out residual risk — the atomic gate prevented future events, but pending events in the transport buffer could still have gone out on the next tick → now `Hub::current().bind_client(None)` hard-drops the transport, buffer content is lost instead of being sent. **DS7 hard requirement fulfilled: "from click onward, nothing more goes out."**
- **F10:** the webapp allowlist still had `ui.route` as a backdoor for later accidental route-tag setting → removed
- **F11:** code-comment drift in `sentry_init.rs` (comment referenced `Hub::end_session()`, code didn't call it) → comment synchronized to the actual `bind_client(None)` implementation

**Round 3 (F12) after 3rd QA pass:**
- **F12:** asymmetric opt-in after opt-out — F9 hard-dropped the client on opt-out, but opt-in only switched the atomic back on without re-binding the client. A pilot who clicked "off → on" had native crash reports broken until an app restart. But the settings hint said "takes effect immediately, no restart needed." Fix: `build_options()` extracted from `init()`, `set_consent(true)` builds a new client via `Client::from(options)` + `Hub::bind_client(Some(...))` + initial scope if none exists. Off-on-off-on now works symmetrically within a single app run, without a restart.

**Round 4 (F13-F15) after 4th QA pass:**
- **F13 (P1):** the F9 implementation was architecturally incomplete — the `SENTRY_GUARD: OnceLock<Option<ClientInitGuard>>` kept an `Arc<Client>` reference permanently alive. `Hub::bind_client(None)` only unbinds from the hub, but the guard kept holding it. Pending events in the transport buffer could have drained via `Client::close()` upon guard drop. The DS7 guarantee was not robust. **Fix:** complete lifecycle overhaul:
  - `SENTRY_GUARD` (=`ClientInitGuard` in the `OnceLock`) removed
  - New `SENTRY_CLIENT: OnceLock<Mutex<Option<Arc<Client>>>>` — we control the Arc reference ourselves
  - `init()` calls `create_and_bind()`: builds `Client::from(options)`, binds via `Hub::bind_client(Some(Arc::clone))`, stores the Arc in the slot
  - `set_consent(false)`: `slot.take()` → our Arc is gone + `client.close(Some(ZERO))` → the transport worker is signaled to discard the pending queue + `Hub::bind_client(None)` → the hub reference is gone. The Arc refcount drops to 0, drop runs, the transport worker thread terminates cleanly.
  - `set_consent(true)`: `create_and_bind()` — no-op if a client already exists, otherwise a new build.
- **F14 (P2):** release notes linked `docs/spec/v0.9.1-*` (doesn't exist — the specs are named `v0.9.0-*`). Fix: all 4 spec links per language reverted to `v0.9.0-*`.
- **F15 (P2):** the known-issue block said "sim-lost suffix is coming in v0.9.1" — but v0.9.1 is exactly this release. Fix: corrected to "coming in a later v0.9.x release (planned v0.9.2)." Since superseded by the F16 implementation.

**Round 5 (F16) after 5th QA pass:**
- **F16 (F7 resolution, P2 → done):** instead of pushing F7 to v0.9.2 as a known issue, it was wired up in v0.9.1 after all. Spec LE8 is thus fully implemented.

**Round 6 (F17) after 6th QA pass:**
- **F17 (P0):** the F13 refactor had used `Client::from(options)` directly, but in doing so omitted `sentry::apply_defaults(options)`. `apply_defaults` sets the default transport (reqwest-based), the default integrations (panic, backtrace, contexts), and env/proxy defaults. **Without this call, the client had NO transport — events didn't go out, native GlitchTip was effectively dead.** The F13 lifecycle fix was OK but half-finished. Fix: `let options = sentry::apply_defaults(options)` before `Client::from()`. Additionally `client.is_enabled()` as a boot log + warn log if false (= visible in journalctl/console if the build config is missing something).
  - New Tauri command `discord_rpc_set_sim_lost(lost: bool)` → calls `Manager::set_sim_lost`
  - Frontend hook `useDiscordRpcPush` has a second useEffect that reacts to `simStatus.state` changes: `lost = simStatus.state !== "connected"`, calls the command. The backend manager deduplicates internally.
  - Applies during an active flight — no flight = no suffix makes sense
  - The known-issue block for this was removed (no open spec items remain in the current release)

## [v0.9.0] — 2026-05-18 · INTERNAL (never published, briefly visible as latest)

Version number **burned** due to a ~15-minute visibility window on `releases/latest` while QA was still running. Content fully contained in v0.9.1. The tag remains in the git log for the audit trail, no pilot distribution.

🚀 **Double-feature release: anonymous error telemetry to a self-hosted GlitchTip + live pilot flight status in the Discord profile. Both features are opt-in, default = off, always toggleable at any time.**

### F-001 · Discord Rich Presence

Pilots can mirror their current flight status live into their Discord profile. Other VA members thus see "Pilot X is flying GSG3184 EDDB→KMRH CRUISE" directly in the member list — without anyone needing to open the pilot client or look into the webapp dashboard.

- **Settings → Discord Rich Presence**: 3 toggles
  - **Master toggle** (default OFF, GDPR opt-in)
  - **Anonymize callsign** ("GSG3184" → "GSG-Flight", route stays visible)
  - **Show "open profile" button** (= phpVMS profile link in the presence)
- **Live status**: green/gray/red dot shows the connection to Discord, updated every 5s
- **Test-presence button**: sends a dummy presence for 15s — the pilot can verify without a real flight
- **20 phases** correctly mapped (no UNKNOWN fallback): Preflight, Boarding, Pushback, Taxi-Out, Takeoff-Roll, Takeoff, **REJECTED-TAKEOFF** ⚠ (v0.10.0-ready), Climb, Cruise, **Holding** (v0.5.11), Descent, Approach, Final, Landing, **GO-AROUND** ⚠ (v0.10.0-ready), Taxi-In, Arrived, Shutdown, **DEBOARDING** (v0.10.0-ready), PIREP-Filed. — Today's Rust FSM emits 17 of these, the 3 v0.10.0 phases are prepared but only fire with the phase-expansion release.
- **60s heartbeat + immediate update on phase change**
- **Graceful fallback**: if Discord is not installed or open → status "NotFound", no crash, no toast spam
- **Takes effect immediately**: toggle off = pipe is closed + activity cleared within 5s

#### Asset layout

- `large_image` = AeroACARS logo (brand consistency)
- `small_image` = sim badge bottom-right (MSFS 2024/2020, X-Plane 11/12 — four dedicated designs with an aviation top-down jet)
- The phase is shown as text in the status line, not as an icon (readable at 30×30 px)

#### Architecture

- **Discord app ID NOT in the client binary** — the VA owner maintains it once in the webapp admin (Settings → Discord → "Discord Application ID"), the pilot client fetches it at runtime via a public endpoint. Advantage: no re-release when the VA switches Discord apps, forks work automatically against their own VPS.
- New Rust workspace crate `discord-presence` (~600 LOC + 24 pure-fn tests)
- Settings persist in `<app_data_dir>/discord_rpc_settings.json` across app restarts

### F-002 · GlitchTip — anonymous crash telemetry

Self-hosted Sentry-compatible error-collection endpoint. AeroACARS (client) + recorder (VPS) + webapp (admin UI) automatically send anonymized crash and error events, so the VA owner sees bugs **before** pilots complain on Discord.

- **Settings → error telemetry (anonymous)**: 1 toggle (default OFF, GDPR opt-in)
- **First-run banner** on the first v0.9.0 start with a clear explanation of what is sent / what is not
- **Privacy guarantees** (GDPR Art. 6 (1) a):
  - **What is sent**: crash stack traces, sim name, aircraft ICAO, app version, OS
  - **What is NOT sent**: position, route, login, IP address, passwords, email
  - **Where to**: the VA's own self-hosted GlitchTip (`tip.kant.ovh`), no 3rd party
- **Tag allowlist + redaction** on both sides (Rust + TS): even if other code accidentally sets PII, it is stripped in the `beforeSend`/`before_send` hook
- **Self-hosted GlitchTip stack** on the VPS: Docker Compose (postgres + redis + web + worker), Caddy with auto Let's Encrypt cert, 4 uptime monitors set up (recorder + GlitchTip self + GSG phpVMS + GSG API)

### Telemetry contract

Both features adhere to `docs/spec/v0.9.0-telemetry-contract.md` (Section 1.3 for the canonical phases, Section 9 for privacy gates).

### Known limitations v0.9.1

- **Phase expansion (REJECTED-TAKEOFF, GO-AROUND, DEBOARDING):** the spec defines it + the Discord mapping exists, but the Rust FSM doesn't emit these phases yet — coming with v0.10.0 (the phase-expansion release per the roadmap). Today 17 of 20 mappable phases are emitted.

*(The "sim disconnected" suffix note mentioned earlier was after all wired up with F7/F16 in round 5 of this release — no longer a known issue.)*

### Miscellaneous

- **Webapp**: the JSONL forensics importer now loads **lazily** instead of automatically — the settings tab opens immediately, the import section shows a "📂 load files now" button
- **i18n**: all new UI strings in DE/EN/IT
- **CI**: the GH Actions release workflow forwards `AEROACARS_SENTRY_DSN` and `VITE_SENTRY_DSN_CLIENT` to `tauri-action` (signed builds have the GlitchTip DSN baked in)
- **VPS deploy**: `deploy-recorder.sh` passes `VITE_SENTRY_DSN_WEBAPP` from the env file through to the Vite build
- **Recorder fix**: `package.json` import in `src/index.ts` via `fs.readFileSync` instead of a static `import-with-json` (previously: tsc threw rootDir at the repo root → dist ended up under `dist/src/`, systemd broke)

---

## [v0.7.17] — 2026-05-12 · Fenix Polish + Bug Bundle

🛠️ **Bug-collection release following tester feedback on v0.7.16. The Fenix profile is now default-on (no more toggle), squawk + aircraft type cleaned up for Fenix, SimBrief refresh kicks in at flight start, the runway-utilization score is finally aircraft-aware, auto-start now says why it doesn't fire.**

### F-001 · Fenix profile from opt-in to default-on

- The beta toggle in Settings has been removed (backend flag `fenix_beta_enabled` gone, Tauri commands out, i18n strings out)
- When a Fenix profile is detected, the LVAR overrides apply automatically (landing/nose/wing light, parking brake etc.)
- The localStorage key `fenix_beta_enabled` is cleaned up on first start — no pilot action needed

### B-001 · Aircraft-type fallback for Fenix

- Before: the activity log showed "Type ?" because Fenix doesn't reliably fill the standard `ATC MODEL` SimVar
- Now: `AircraftProfile::icao_fallback()` sets the ICAO code from the profile match for FenixA319/A320/A321 (`A319`/`A320`/`A321`)
- Profiles without a distinct variant (default, FBW, PMDG) keep `None` — no fantasy ICAOs

### B-002 · Squawk logging suppressed for Fenix

- The standard `TRANSPONDER CODE:1` SimVar is not synced with the cockpit-side RMP on Fenix (= showed wrong / frozen codes)
- Until a Fenix-specific LVAR is identified, the snapshot now returns `transponder_code: None` for `is_fenix()` → no more incorrect squawk entries in the activity log and PIREP

### N-001 · SimBrief refresh now kicks in at flight start

- Before: the pilot presses "Refresh" in the bid tab (shows fresh SimBrief data), clicks flight start → `flight_start` ignores that and fetches the old OFP from the phpVMS bid pointer
- Now: if the pilot has set an identifier (user ID or username) in Settings → SimBrief, `flight_start` **first** fetches the most current OFP directly from simbrief.com (with DEP/ARR match verification); fallback to the bid pointer only if direct fetch fails
- Identical behavior to the `flight_refresh_simbrief` path — direct-first with pointer fallback

### N-002 · Runway-utilization score aircraft-aware

- Before: the rollout thresholds were absolute meters (800/1200/1800/2500) → every airliner with a 2 km rollout got "long_rollout" / 25 pts, even though 2 km is perfectly normal for an A320
- Now: 3 aircraft categories (light / medium / heavy) with adjusted thresholds:
  - Light (default): unchanged
  - Medium (A32x family, B737, E170/190, CRJ, ATR, Dash-8 etc.): 1200/1800/2400/3000 m
  - Heavy (A330/340/350/380, B747/767/777/787, MD11): 1500/2300/3000/3800 m
- Aircraft classification via ICAO type-designator lookup, robust against whitespace / case
- Both places (pilot-client Rust crate AND aeroacars-live webapp) need fixing in sync — this version fixes the pilot client; the webapp repo gets a separate patch

### N-003 · Auto-start now says why it doesn't fire

- Before: 3 silent skip paths (`sim_data_warm`, `bids empty`, `no_bid_match`) → the pilot was left puzzled, the watcher only logged debug
- Now: all skip paths have an activity-log hint with a 60-second throttle:
  - **Aircraft title missing** (X-Plane specific: "enable web API in Settings → Network"; MSFS: "sim still booting")
  - **Fuel = 0** (sanity threshold loosened from 100 kg to 1 kg, so light GA with a half-full tank isn't excluded)
  - **No bids available** ("logged in?")
  - **No bid matches current position** (with distance to the nearest departure)
  - **`flight_start` failed** (bid + error code as a warn entry)

### N-004 · X-Plane plugin version sync

- `xplane-plugin/CMakeLists.txt` had carried `VERSION 0.5.0` since the initial commit, while `plugin.cpp` went through 6 patches (v0.5.3/.5.6/.5.8/.5.11/.5.13)
- The plugin therefore falsely logged "v0.5.0" in X-Plane's `Log.txt` — confusing for bug reports
- Now: `VERSION 0.5.13` (= the real code state), pulled into the plugin as the macro `AEROACARS_PLUGIN_VERSION` via `target_compile_definitions` → the log reports the truth
- To be kept in sync with code changes going forward

### Tests

- 15 new Rust unit tests (sim-core: 1 icao_fallback, sim-msfs: 5 Fenix mapping + 3 ICAO fallback + 2 squawk suppression; landing-scoring: 4 runway-utilization cases)
- `cargo test --workspace --lib`: all green
- `tsc -b` clean, `npm test` green

### Guarantees

- F-001: non-Fenix aircraft are unchanged (only the profile check routes into the override)
- B-001/B-002: only apply on an `is_fenix()` profile match
- N-001: SimBrief-direct only when an identifier is set; otherwise the bid-pointer path as before
- N-002: light thresholds identical to v0.7.16 → no regression for GA pilots
- N-003: activity-log spam prevented by the existing 60-second throttle per reason code

### Tracker

See [docs/qs/v0.7.16-fenix-beta-bugs.md](docs/qs/v0.7.16-fenix-beta-bugs.md) for the complete bug collection and diagnostic traces from tester feedback.

---

## [v0.7.16] — 2026-05-12 · Fenix A32x Cockpit State (Opt-in Beta)

🧪 **Stable release with a new opt-in beta feature for the Fenix A32x. Disabled by default. Read-only, no FSUIPC, no MSFS Community-folder changes. Anyone who doesn't enable it flies bit-identically to v0.7.15.**

### What this version delivers

#### Fenix A319 / A320 / A321 variant detection
- `AircraftProfile` extended with `FenixA319` and `FenixA321` (previously both ran as `FenixA320`)
- Detection via title substring + ICAO fallback (for repaints without a variant suffix)
- Helper `AircraftProfile::is_fenix()` for all three variants
- Label differentiation: "Fenix A319" / "Fenix A320" / "Fenix A321"

#### Additive Fenix LVAR mappings (opt-in)
New under the `fenix_beta_enabled` flag (default off):
- `L:S_OH_EXT_LT_LANDING_L` + `_R` (3-position selector: retracted/off/on) → `light_landing`
- `L:S_OH_EXT_LT_NOSE` (3-position: off/taxi/T.O.) → `light_taxi`
- `L:S_OH_EXT_LT_WING` (wing inspection) → new: `light_wing` for Fenix-beta users
- `L:S_OH_EXT_LT_RWY_TURNOFF` (runway turnoff, read-only QA)
- `L:S_OH_EXT_LT_LANDING_BOTH` (composite, verified against L/R)
- `L:S_FC_FLAPS` (flaps-lever detent, read-only QA)

LVAR names verified against the **real Fenix install** on the dev machine — source:
`SimObjects\Airplanes\FNX_32X\model\FNX32X_Interior.xml` in
`fnx-aircraft-320` / `fnx-aircraft-319-321`.

#### Feature-flag infrastructure
- `fenix_beta_enabled: AtomicBool` on `MsfsAdapter::Shared`
- Tauri commands `set_fenix_beta_enabled` / `get_fenix_beta_enabled`
- Frontend toggle in Settings → Beta (localized DE / EN / IT)
- localStorage persistence + backend sync on app mount
- Default off → bit-identical behavior to v0.7.15 stable for all non-beta users

### Spec guarantees

- ❌ No writes, no control, no FMC access
- ❌ No FSUIPC dependency
- ❌ No MSFS Community-folder additions (no WASM module, no DLL)
- ✅ Read-only via plain SimConnect with `L:` prefix
- ✅ The pilot only needs to install the AeroACARS update + flip the switch
- ✅ On a missing LVAR: silently fall back to the standard MSFS SimVar, no crash

### Tests

- 12 new Rust unit tests (7 in sim-core for A319/A320/A321 detection + is_fenix helper; 5 in sim-msfs for beta-on/off mapping + layout smoke test)
- `cargo test --workspace --lib`: 224/224 passed

### Verification

| Check | Status |
|---|---|
| `cargo check` (client/src-tauri) | ✅ |
| `cargo test --workspace --lib` | ✅ 224/224 |
| Spec path update (`Cockpit_Behavior.xml` → `FNX32X_Interior.xml`) | ✅ |
| LVAR names vs. real Fenix install | ✅ |
| Stable behavior (beta off) bit-identical to v0.7.15 | ✅ |

### Release rules

This version ships **as a normal stable release** to all pilots via the auto-updater. The Fenix profile is opt-in and default off, so there's no risk for the broad user base.

Stable adoption of the Fenix LVAR mappings into the **default path** (= without opt-in) will follow at the earliest once the beta feedback is positive.

### Documents

- Spec: [docs/spec/fenix-a32x-cockpit-state-beta.md](docs/spec/fenix-a32x-cockpit-state-beta.md)
- QA guide: [docs/spec/fenix-a32x-beta-qs-guide.md](docs/spec/fenix-a32x-beta-qs-guide.md)

---

## [v0.7.15] — 2026-05-12 · Sim-Recovery Release

🎯 **Ongoing flights now cleanly survive a simulator crash, pause, restart, or brief computer interruptions — no data hole, no session split, with a correct flight time.**

### Trigger

Real pilot incident **AUA 323 LOWW→ESGG on 2026-05-11** (PIREP `J2VoaZmoD6LQGpMg`): MSFS froze during descent, ACARS paused after 30 s, the pilot only noticed after landing manually on the ground in ESGG. Result: two sessions in the history tab, block-time drift, the AeroACARS recorder didn't bridge a 23-minute gap.

This version is a **combined sim-recovery release** that tackles this root cause in three places: pause handling, session identity, sim awareness.

### What this version delivers

#### Phase 1 (client) — auto-resume + pause accumulator + `pirep_id` payload
- The streamer loop resumes automatically as soon as sim data comes in again — no more manual "resume flight" click needed
- The manual resume button remains as a fallback
- Pause durations are accumulated in `pause_total_duration_secs` + appended to `pause_segments` (audit log)
- The block/flight time in the PIREP deducts accumulated pause time (heartbeat + file + manual edit)
- The reposition distance on resume is NOT added to `distance_nm` (`last_lat`/`last_lon` reset)
- `pirep_id` is sent along in every position MQTT payload
- **Heartbeat fix**: on a sim disconnect without a snapshot, a heartbeat with `last_good_snap` is still sent to phpVMS every 30 s → the PIREP stays alive indefinitely

#### Phase 2 (server) — `pirep_id` join in `ensureSession`
- The recorder prioritizes `pirep_id` from the payload OVER the standard heuristic (callsign/dep/arr + time window) → 23-minute position gaps no longer create a new session
- 6h cutoff: ENDED sessions can only be reopened within 6h of `last_seen`
- Terminal protection: ARRIVED/PIREP_SUBMITTED sessions are NEVER reopened
- Backfill: ACTIVE sessions without `pirep_id` get one attached retroactively
- Freshly created sessions get `pirep_id` set directly

#### F5 — MSFS pause via SimConnect `Pause_EX1`
- Subscribes to the SimConnect system event `Pause_EX1` with the `dwData` flag set
- Active MSFS Esc pause / active pause / sim pause are recognized immediately — no more 30 s wait on the disconnect threshold
- Reliable initial state: if AeroACARS connects while MSFS is already paused, an initial Pause_EX1 event arrives immediately
- `SimSnapshot.paused` is passed through, the streamer pauses + accumulates
- Auto-resume on a Pause_EX1 event with `dwData=0` without a pilot click

#### F6 — X-Plane pause + replay mode
- New RREF subscriptions on `sim/time/paused` + `sim/time/is_in_replay`
- `SimSnapshot.paused` is fed from both (replay mode counts as pause-equivalent)
- Works without an X-Plane plugin update — RREF is native, no protocol bump needed

#### F7 — Aircraft-change warning after recovery (MSFS + X-Plane ≥12.1)
- On resume, compares `snap.aircraft_icao` vs. `flight.aircraft_icao` (bid value)
- On mismatch: activity-log warning with a specific hint ("sim reports A320, bid expects B738")
- Resume is NOT blocked (spec principle P2: inform instead of block) — the pilot can correct via the PIREP-cancel UI
- **Sim coverage:** MSFS via SimConnect (ATC MODEL); X-Plane via web API from v12.1 (`sim/aircraft/view/acf_ICAO`). X-Plane <12.1 or with the web API disabled silently skips F7 (no false-positive warnings)

### Data model

`FlightStats` + `PersistedFlightStats` extended with (all `#[serde(default)]`, forward-only):
- `pause_total_duration_secs: i64` — sum of all pause seconds
- `pause_segments: Vec<PauseSegment>` — audit data per pause block (start, end, reason, drift)
- `current_pause_reason: Option<PauseReason>` — active reason for the resume helper
- `PauseReason` enum: `SimDisconnect` | `SimPause` | `ManualResume`

Pre-v0.7.15 `active_flight.json` continues to load — missing fields default to `0` / an empty Vec / `None`.

### Tests

- 16 new Rust unit tests (block-time saturating arithmetic, drift-threshold monotonicity, PauseSegment serde roundtrip, PausedFlightStats backward compat, SimPause-reason persistence)
- 5 Node test-driver tests in the recorder (25-min gap, terminal protection, 6h cutoff, legacy backfill, brand-new session)
- `cargo test --lib`: 115/115 passed
- `npm test` in the recorder: 5/5 passed

### Verification

| Check | Status |
|---|---|
| `cargo check` (client/src-tauri) | ✅ |
| `cargo test --lib` | ✅ 115/115 |
| `npx tsc --noEmit` (recorder) | ✅ |
| `npm test` (recorder) | ✅ 5/5 |
| forward-only: pre-v0.7.15 `active_flight.json` loads | ✅ via `serde(default)` |

### Companion server deploy

Server patches are pushed to `aeroacars-live` (commits `92b22c6` + `0ffceca`). Deploy to `live.kant.ovh` via `deploy-recorder.sh`.

### Deliberately out of scope (coming later)

- F8 bid-change detection (only a light check had been planned, not finished)
- New toast/banner UI architecture
- Drift line on the map
- Full 29-scenario pilot QA matrix

### Spec reference

Complete requirements + acceptance criteria: [`docs/spec/sim-disconnect-auto-resume.md`](docs/spec/sim-disconnect-auto-resume.md)

Trigger-incident data (PIREP `J2VoaZmoD6LQGpMg`) for later forensic review.

---

## [v0.7.14] — 2026-05-12

🎯 **Discord posts now run centrally from the VPS — the pilot client no longer posts anything.**

### Why

v0.7.13 introduced a pilot-local webhook URL — the pilot pasted the URL into Settings. Problem: with N pilots = N places where the token could leak + every pilot could set a different URL (= Discord-spam risk).

In addition, the recorder on live.kant.ovh has **already had its own Discord integration for months** (webapp admin → Settings → Discord webhook for touchdown + PIREP posts). The pilot client posted additionally → duplicate posts for landing + PIREP, plus two individual events (takeoff, divert) that only the pilot client posted.

v0.7.14 cleans this up: **pilot-client Discord code completely removed, the recorder is the only source**.

### Pilot client (~250 LOC removed)

- `client/src-tauri/src/discord.rs` deleted completely
- `mod discord;` removed from `lib.rs`
- 4 `discord::post_event(...)` calls in `lib.rs` removed (takeoff, landing, PirepFiled, divert)
- `discord_webhook_get` + `discord_webhook_set` Tauri commands removed
- Settings section "Discord integration" + i18n keys (DE/EN/IT) removed
- Migration: the old `<app_data_dir>/discord-webhook.txt` from v0.7.13 is automatically deleted on first start under v0.7.14
- The Cargo `discord-rich-presence` dep had already been removed (v0.7.13)

### Recorder (~80 LOC new)

- `postTakeoff(db, ev)` — new Discord poster for takeoff events. Triggered by the MQTT `takeoff` channel that the recorder already receives.
- `postPirep` extended with divert detection: on `payload.divert === true`, the embed shows `🔀 DIVERT filed` (orange) with a clear before→after route (`EDDF → ~~MDPC~~ ➜ MDST`)
- `mqttSubscriber` new `onTakeoff` hook
- `index.ts` wires `void postTakeoff(db, row)` to the subscriber
- New setting `enable_takeoffs` (default: true) — same pattern as `enable_touchdowns`/`enable_pireps`

### Webapp (admin)

- Settings → Discord webhook: additional toggle "post takeoffs" between the webhook URL and "post touchdowns"
- PIREPs label extended: "post PIREPs (incl. divert embed on diversions)"

### What the VA owner does (one-time)

1. Browser: https://live.kant.ovh/admin/ → Settings → Discord webhook
2. Paste in the **webhook URL** (from the Discord server, if not already there)
3. Check the **toggles** (takeoffs/touchdowns/PIREPs all ✓ is the default)
4. **Test** button → green "✓ webhook OK"
5. **Save**

Done. **No pilot needs to do anything.** On the next flight, the recorder automatically posts takeoff + touchdown + PIREP (+ divert if applicable).

### Security properties

- The URL lives **only** in the recorder's SQLite DB (`/var/lib/aeroacars-recorder/`)
- The URL **never** goes to a pilot client
- The pilot can't see, change, or misuse the URL
- Rotation: the VA owner changes the URL in 30 sec in the webapp admin, done

---

## [v0.7.13] — 2026-05-12

🧹 **Codebase audit + security cleanup — no more hardcoded token, ~700 LOC dead code removed.**

### Background

Complete QA audit across both codebases (pilot client + aeroacars-live + cross-cutting security). 3 parallel auditor agents identified **18 items**. This release picks out all **pilot-client-relevant** items (= SSH/VPS changes were deliberately excluded for a later release).

### Critical fix

**A1 — Discord webhook token no longer hardcoded** (audit C1)
- `discord.rs:27` had the GSG Discord webhook token in plain text. The repo is public on github.com/MANFahrer-GF/AeroACARS → the token was effectively public.
- v0.7.13 now reads the webhook URL from 3 sources (in priority order): env `AEROACARS_DISCORD_WEBHOOK` > `<app_data_dir>/discord-webhook.txt` (chmod 0600) > None.
- Settings → Discord integration: new field "webhook URL" where the pilot/VA owner pastes the URL.
- Default = empty = no posts. The pilot must actively configure it.
- **Important for VA owners:** rotate the old webhook in Discord, create a new one, distribute the URL to pilots (pinned post on Discord or PM).

### Cleanup (~700 LOC dead code removed)

| # | What | Location |
|---|---|---|
| B7 | Discord `EventContext.airline_icao` + `fuel_used_kg` fields + 4 setters | `discord.rs:52` + `lib.rs` × 4 |
| B6 | `current_premium_status` + `pirep_queue::count` Cargo warnings | `lib.rs:7230, 12241` |
| B5 | `fcu_debounce()` + 8 Fenix FCU state fields (plan discarded) | `lib.rs:2198, 16331` |
| B4 | 6 orphan Tauri commands without a frontend caller | `landing_get`, `get_minimize_to_tray`, `get_simbrief_settings`, `ofp_callsign_warning_dismiss`, `xplane_uninstall_plugin`, `detect_running_sim` |
| B3 | 4 orphaned React components | `Dashboard.tsx`, `FlightInfoPanel.tsx`, `MassPanel.tsx`, `PhaseTimeline.tsx` |
| B2 | Discord-rich-presence block (~170 LOC dead since v0.4.0) | `discord.rs:485-659` + `discord-rich-presence` Cargo dep |
| C1 | `secrets::migrate_from_keyring()` + `keyring` Cargo dep (v0.5.15 migration, 30+ releases ago) | `crates/secrets/src/lib.rs:183` |
| C2 | 4 dead i18n keys + the entire `dashboard:` locale block (DE/EN/IT) | `tabs.dashboard`, `landing.peak_vs`, `landing.plan_tow`, `landing.plan_ldw` |
| C3 | Workspace dep `schemars = "0.8"` without a code path | `Cargo.toml` |
| C4 | Stale "wiring coming in v0.4.5" + "patch in v0.7.7" comments | various |

### Security hardening

| # | What |
|---|---|
| B1 | The Tauri updater private key moved from `client/aeroacars-updater.key` to `~/.aeroacars-keys/` (was in `.gitignore`, but one `git add -f` = catastrophic). GitHub Actions secrets not affected. |
| C5 | The Tauri webview CSP set from `null` to strict (audit M-Sec-7). `default-src 'self'` + explicitly allowed `connect-src` (phpVMS via `https:`, MQTT WS `wss://live.kant.ovh`, SimBrief). XSS defense-in-depth. |

### Documentation + audit traces

| # | What |
|---|---|
| D1 | `cargo audit` run locally → 4 vulnerabilities in `rustls-webpki@0.102.8` via `rumqttc 0.24 → rustls 0.22` → documented in the audit report, dep-tree update for a later release |
| D2 | MEMORY.md corrected: secrets storage is **file-based** (`<app_data_dir>/secrets.json`, chmod 0600), NOT OS keyring (documentation had been wrong for 30 releases) |
| D3 | 8 stale specs (all marked as "Approved" / "CODE-READY") archived to `docs/spec/historical/`. Only `requirements.md` stays active |
| D4 | Audit reports (`pilot-client-audit.md`, `aeroacars-live-audit.md`, `security-audit.md`, `MASTER-AUDIT-REPORT.md`) committed to the repo |

### What is NOT in v0.7.13 (deliberately, for a separate release)

- **C2** (`NOPASSWD:ALL` on the VPS) → needs an SSH patch + testing the admin endpoints
- **H1+H2** rate limit on `/api/login` + re-auth on admin endpoints → recorder code change
- **H3** `@fastify/static` major bump to 9.x → recorder testing
- **H5** `bcrypt → bcryptjs` → recorder migration
- **Caddy security headers** (CSP, HSTS-explicit, X-Frame-Options)

→ These 5 items land in the next VPS-side release once you free up an SSH window for testing.

### Cargo-audit finding (pending)

4 vulns in `rustls-webpki@0.102.8` (transitive via `rumqttc`). Solution = `rustls-webpki >=0.103.12` but that requires a `rumqttc` upgrade to a version using `rustls 0.23+` → an API-breaking check needed. Noted in `docs/audit/MASTER-AUDIT-REPORT.md` Q1. Planned for v0.7.14.

---

## [v0.7.12] — 2026-05-12

🐛 **Bid card: pax + cargo appear again — even without a phpVMS bid pointer.**

### Background

Reported right after the v0.7.11 release: for CFG2228 (Condor bid), there was an empty area between the aircraft line and the SimBrief plan block — the pax/cargo chips were completely missing. Cause: the v0.7.10 pre-flight SimBrief-direct fetches the OFP directly from simbrief.com without populating the phpVMS bid-pointer subfleet fares. If the bid didn't yet have an OFP bound via phpVMS (or phpVMS has no fare entries for this subfleet), `paxCount` and `cargoKg` were 0 → chips hidden.

### Fix

- `api-client/lib.rs` SimBrief XML parser: now extracts `<weights><pax_count>`
  + `<weights><pax_count_actual>` (fallback) + `<weights><cargo>` (kg).
- `SimBriefOfp` struct: new fields `pax_count: i32` + `cargo_kg: f32`.
- `BidSimBriefPreview` (Tauri response): the same fields added.
- `bid_simbrief_preview` command: populates the values from the parsed OFP.
- `BidsList.tsx` BidDetails: `paxCount`/`cargoKg` now prefer the
  preview values (> 0) over the bid subfleet fares. The fallback thus stays
  identical to the v0.3.0 behavior for pilots without SimBrief settings.

---

## [v0.7.11] — 2026-05-12

🎯 **One sink rate, one truth — an end to the jungle of values.**

### What

Real pilot case: DAL804 showed `-407 fpm` in phpVMS/AeroSore but `-364 fpm` in the AeroACARS UI. Both values were produced by the pilot client — one from the MSFS SimVar `PLANE TOUCHDOWN NORMAL VELOCITY` (latched), the other from the 50 Hz buffer edge (interpolated). The pilot was understandably irritated ("the pilot has already called me stupid").

v0.7.11 makes the 50 Hz buffer-edge value (`vs_at_edge_fpm`) the **sole canonical score basis** and cleans up the UI so no more confusing parallel values appear anywhere.

### Backend (Rust)

- `lib.rs` buffer-dump hook: if the v2 touchdown-forensics cascade (`vs_at_impact → smoothed_500ms → smoothed_1000ms → pre_flare_peak`) REJECTs the primary VS value, **`vs_at_edge_fpm`** (50 Hz on-ground-edge interpolation) is now used instead of the MSFS SimVar. The SimVar-latched value thus drops out of the score, the MQTT payload, and the phpVMS PIREP.
- `finalize_landing_rate(stats, vs_fpm, confidence, source)` atomic-write helper ensures that `landing_rate_fpm`, `landing_rate_confidence`, and `landing_rate_source` are ALWAYS set together.

### Pilot client (frontend)

- `LandingPanel.tsx` touchdown card cleaned up: all smoothed-VS variants (250/500/1000/1500 ms), `vs_at_edge_fpm`, `landing_peak_vs_fpm`, `peak_g_post_500ms/1000ms` removed from the touchdown card. The pilot now only sees ONE sink rate here (= the score basis), touchdown G, peak G, pitch/bank/speed/sideslip, bounces, heading. The smoothed values continue to live in the **sink-rate forensics section** (v0.7.8) — that's where they belong.

### VPS webapp

- The "algorithm forensics" sub-section in the diagnostics card removed. It showed VS-estimator comparisons (Lua-30 / time-tier-MSFS / SimVar-final-VS) — belongs in internal backend logs, not the pilot UI. The 50 Hz TouchdownWindow card below continues to show all relevant forensic values for the VA owner.

### Before/after

- v0.7.10: DAL804 → phpVMS `-407 fpm` (SimVar) / AeroACARS UI `-364 fpm` (50 Hz edge) → pilot confused
- v0.7.11: DAL804 → phpVMS `-364 fpm` / AeroACARS UI `-364 fpm` → consistent

---

## [v0.7.10] — 2026-05-11

✨ **Pre-flight SimBrief-direct: fresh OFP values already in the bid list — before the IFR start.**

### What

Previously, the pilot could only load SimBrief OFP data after the IFR start via "refresh" — before that, the bid list only showed the phpVMS pointer values (often outdated). Pilot wish: *"why can't I get the data before the IFR start already?"*

v0.7.10 fetches SimBrief data directly from simbrief.com (via the `bid_simbrief_preview` Tauri command) as soon as the bid list is loaded.

### Backend (Rust)

- New `#[tauri::command] bid_simbrief_preview(bid_id: i64)` — fetches the SimBrief OFP for a bid directly from simbrief.com, without the `phase_locked`/`active_flight` gate
- Reuses the `try_simbrief_direct_with_match` logic (same code path as the refresh button)

### Frontend

- `BidsList.tsx` `fetchPreviewsForBids()` fetches all previews in parallel when the bid list loads
- Green "✓ fresh SimBrief values loaded" banner on success
- New notice tone `"ok"` (in addition to `info/warn/err`) — green CSS

---

## [v0.7.9] — 2026-05-11

🔄 **SimBrief OFP refresh: callsign match switched to a SOFT warning — DEP/ARR is the real anchor.**

### What

Real pilot case: phpVMS bid `EWL9725` (Eurowings Europe operator code EWL), SimBrief OFP callsign `EWG4PY` (Eurowings ICAO + personal callsign). DEP/ARR identical (LOWG → EDDL), but the v0.7.7 match verification rejected the OFP as a "mismatch" → the refresh button got no new OFP data, the pilot saw nothing and was frustrated.

v0.7.9 turns the callsign check into a SOFT warning instead of a hard block:

- **DEP+ARR match** → the OFP is **loaded** (previously a hard block on a callsign diff)
- **Callsign differs** → the OFP is still loaded, plus a yellow warning notice with concrete values ("OFP callsign is EWG4PY, your active flight uses EWL9725 / 9725 — DEP/ARR match, OFP was loaded")
- **DEP or ARR differ** → the hard block remains (= clearly a different flight)

### Backend (Rust)

- New `DirectOutcome::MatchWithCallsignWarning { ofp, simbrief_callsign }` variant between `Match` and `Mismatch`
- `try_simbrief_direct_with_match` two-stage logic: DEP/ARR first (hard), then callsign (soft)
- New `AppState::ofp_callsign_warning: Mutex<Option<OfpCallsignWarning>>` — mirrored to the frontend via:
  - `#[tauri::command] ofp_callsign_warning_get()` — frontend reads it after a refresh
  - `#[tauri::command] ofp_callsign_warning_dismiss()` — the X button clears it
- A clean match clears the warning automatically (otherwise it hangs around after a new refresh)

### Frontend

- `BidsList.tsx` `refreshAll()` polls the warning after `flight_refresh_simbrief` and renders a yellow notice
- New i18n key `flight.ofp_callsign_warning` in DE/EN/IT with placeholders `{{sb_callsign}}` + `{{active_callsigns}}`

### What stays unchanged

- The v0.7.7 pointer-path fallback (on `bid_not_found`)
- The DEP+ARR hard match (only the callsign switched from hard to soft)
- The SimBrief settings configuration (Settings → SimBrief integration)

---

## [v0.7.8] — 2026-05-11

🎯 **Sink-rate forensics in the landing tab — the pilot understands why their landing rate is what it is.**

### What

Pure UI extension, no new data collection. Addresses recurring pilot complaints of the type *"Volanta shows me 232 fpm but AeroACARS scores 357 — who's right?"* — both values are physically correct, they measure different things. The new section explains this transparently.

Spec: `docs/spec/v0.7.8-landing-rate-explainability.md` v1.8 APPROVED (8 QA iterations).

### What changes (in the landing tab)

New section **🎯 Sink-Rate Forensics** between approach stability and flare quality, with 6 blocks:

1. **Explanation block** (cyan accent): "Which sink rate is the right one?" — explains that Volanta/cockpit VSI shows an average over 0.5-1.5 s, while AeroACARS scores the cascade value directly at the touchdown moment (FAR 25.473 engineering standard)
2. **Tool-average tiles** (4 tiles): 1.5 s / 1.0 s / 0.5 s / 0.25 s from `vs_smoothed_*_fpm` — what your cockpit/Volanta typically displays
3. **Bucket breakdown**: disjoint bucket differences from the 4 cumulative averages — shows how the sink rate developed in each phase. On a monotonic increase in magnitude (|Δ| > 20 fpm across all 3 inter-bucket steps): auto-trend note "flare not held / sagged through"
4. **Score-basis tile** (large + prominent): `landing_peak_vs_fpm ?? landing_rate_fpm` with tone color per `T_VS_*` bands (200/400/600/1000 fpm) + `landing_source` as a source pill
5. **Coaching tip** (one sentence by priority): flare_lost / hard_g / no_flare / late_drop / clean
6. **More details** (collapsible, default collapsed): position trace of the last 3 s from `touchdown_profile` (NOT `approach_samples` — that only has vs_fpm/bank_deg) + impact load (peak G post-TD 500ms/1s)

### Backwards compat

- The section renders if `hasForensics(record) === true` — at least one of `forensic_sample_count`, `vs_smoothed_*_fpm`, `vs_at_edge_fpm` set
- Entries without 50 Hz forensics data show a compact legacy hint ("Forensics data was not yet stored for this older flight")
- Tiles with a `null` value show `—` (em-dash), grid stays a stable 2x2
- The score-basis source pill only when `landing_source != null && !== ""` (pre-v0.7.1: no pill, no error)

### Design consistency with the AeroACARS look (§4.5)
</content>
