# Touchdown-Forensik v2 — Architektur-Spec für v0.7.0

**Status:** v2.3 — **Approved for implementation** (VA-Owner sign-off after 3 review rounds, commit 9e663c2)
**Cutoff:** Forward-only — gilt für Flüge mit `forensics_version: 2` Marker (rolled out ab dem v0.7.0-Release).

---

## Changelog v2.0 → v2.1 → v2.2 → v2.3

### v2.3 (3 P2/P3 Konsistenz-Korrekturen aus drittem Review)

| # | Issue | Fix | Sektion |
|---|---|---|---|
| **P2.3-A** | PTO 705 Beispiel sagte „B1+B2+B3+B4 all pass", aber B2 verlangt ≥500ms und 307ms ist FAIL. Validator-Implementierer würde verwirrt | PTO 705 Beispiel zeigt jetzt explizit B2=FAIL, andere PASS, 3/4 → VALIDATED via Voting. Confidence auf Medium statt High | 6.3 |
| **P2.3-B** | load_peak_window war [contact, contact+500ms], aber DAH-Beispiel nannte 1635kN @ 5.7s nach contact (= ausserhalb window) | Trennung: `initial_load_peak` (im 500ms window) + `episode_load_peak` (ganze Episode). DAH-Beispiel zeigt beide klar | 5.1, 6.4 |
| **P3.3** | `partial_cmp(...).unwrap()` Pseudocode kann bei NaN paniken | NaN-safe via `is_finite()` filter + `total_cmp()` statt `partial_cmp().unwrap()` | 5.2 |

### v2.2 (3 P1/P2 Punkte aus zweitem Review + Nebenpunkte)

| # | Issue | Fix | Sektion |
|---|---|---|---|
| **P1.2** | impact_frame Widerspruch — Definition `min vs in window` ergibt -414 fpm, Acceptance hatte fälschlich -334 | DAH 3181 Acceptance auf vs ∈ [-415, -395] fpm korrigiert, Score = **firm** (nicht acceptable). Dokumentiert: -334 ist load-transfer (sample 200), -414 ist raw min vs (sample 197), -401 ist vs_at_contact_edge (sample 198) | 5.4, 10 |
| **P2.2** | `total_weight_kg` fehlt im Replay-Schema | `total_weight_kg: Option<f32>` in TouchdownWindowSample aufgenommen. Plus snapshot im LandingEpisode für deterministische Replays | 7.1, 6.1 |
| **P3.2** | gear_force-Confirmation wieder sample-basiert | Komplett zeitbasiert: „Force über Threshold für ≥ 60ms anhaltend (gemessen via Timestamps), mindestens 2 distinct samples zur Anti-Glitch-Sicherung" | 4.4 |
| **N1** | PTO 705 hat 2 low-level touches im T&G-Fenster, nicht 1 | Verifiziert: Periode 1 (307ms, vs=-182), Pause 2.3s, Periode 2 (339ms, vs=-61). LandingEpisode-Beispiel + Acceptance dokumentiert beide | 6.3, 10 |
| **N2** | Übergangsphase Event-Renaming | Legacy `touchdown_complete` Event wird aus `landing_finalized` abgeleitet für eine Übergangsphase (1-2 Releases). Alte Recorder/Web-Versionen brechen nicht | 7.2 |

### v2.1 (13 Punkte aus erstem Review)

[siehe Anhang A — vollständige v2.0→v2.1 Diff-Tabelle ans Ende]

---

## 0. Warum dieses Dokument

Das aktuelle Touchdown-Forensik-System (v0.5.x → v0.6.2) hat **9 zusammenhängende Bugs** gleicher architektonischer Wurzel:
- Single-shot TD-Detection (= „first edge wins")
- Sim-Engine-edge wird als „echter" TD verwendet (X-Plane edge-trigger-happy bei Float)
- vs_at_edge unconditional-override ohne Plausibilitäts-Prüfung
- Keine Multi-TD-Unterstützung (T&G/Go-Around/Bounce)
- Keine Confidence-Tagging

**Beweis aus echten Flügen** (alle 6 test-flights mit voll-funktionierendem 50Hz Buffer post v0.6.0):

| Flug | Sim | Heute angezeigt | Was richtig wäre | Δ |
|---|---|---|---|---|
| PTO 105 GA | MSFS | -55 fpm / 100 (smooth) | -55 fpm / smooth | 0 ✓ |
| **PTO 705 T&G** | MSFS | -182 fpm vom Streifschuss | 2 Episoden: Ep 0 = T&G mit 2 low-level touches; Ep 1 = FinalLanding mit eigenem Score | unbekannt ❌ |
| DLH 304 | MSFS | -357 fpm / 80 (acceptable) | -357 fpm / acceptable | 0 ✓ |
| CFG 785 | MSFS | -142 fpm / 100 (smooth) | -142 fpm / smooth | 0 ✓ |
| DLH 742 | MSFS | -191 fpm / 100 (smooth) | -191 fpm / smooth | 0 ✓ |
| **DAH 3181** | **X-Plane** | **+104 fpm / 80 (acceptable)** | **vs ∈ [-415, -395] fpm / firm** | **~ -510** ❌ |

**Befund:** MSFS-Pfad ist algorithmisch korrekt (4/4 Flüge unverändert). X-Plane-Pfad ist algorithmisch broken. T&G/Go-Around-Pfad ist broken sim-übergreifend.

**Verifiziertes DAH 3181 Sample-Trace (mit korrekten VS-Werten):**

```
Sample 124 (07:53:58.43)  on_ground=False  agl=2.79  vs= -44   ← in Luft
Sample 125 (07:53:58.46)  on_ground=True   agl=2.60  vs=+104   ← Edge 1 (Float-Streifschuss)
Sample 126 (07:53:58.51)  on_ground=True   agl=2.62  vs=+162
Sample 127 (07:53:58.54)  on_ground=False  agl=2.79  vs=+207   ← schon wieder in Luft (Dauer: 44ms)
…
Sample 193 (07:54:02.06)  on_ground=False  agl=5.23  vs=-371   ← Final-approach descent
Sample 194 (07:54:02.11)  on_ground=False  agl=4.54  vs=-391
Sample 195 (07:54:02.17)  on_ground=False  agl=3.83  vs=-406
Sample 196 (07:54:02.24)  on_ground=False  agl=3.12  vs=-413
Sample 197 (07:54:02.28)  on_ground=False  agl=2.78  vs=-414   ← MIN VS (raw härtester Sink-Moment)
Sample 198 (07:54:02.31)  on_ground=True   agl=2.46  vs=-401   ← contact_frame (vs_at_edge)
Sample 199 (07:54:02.36)  on_ground=True   agl=2.46  vs=-401
Sample 200 (07:54:02.40)  on_ground=True   agl=1.82  vs=-334   ← load-transfer / suspension dampening
…
```

→ Drei VS-Kandidaten, alle aus dem 50Hz-Buffer:
- **vs_at_edge** (sample 198) = -401 fpm — Volanta-style
- **vs_min in window** (sample 197) = -414 fpm — raw härtester Touch
- **vs_at_load_transfer** (sample 200) = -334 fpm — bereits gedämpft

Spec wählt **`vs_min in window` als impact_frame** (siehe 5.1) → finaler Score ≈ -414 fpm.

---

## 1. Daten-Inventar (was wir HABEN, was wir NICHT haben)

### 1.1 Pro 50Hz Sampler-Sample (heute, im JSONL `touchdown_window`-Event)

```
at, vs_fpm, g_force, on_ground, agl_ft, heading_true_deg,
groundspeed_kt, indicated_airspeed_kt, lat, lon, pitch_deg, bank_deg
```

### 1.2 Pro Streamer-Position-Snapshot (1-3s cadence, im JSONL `position`-Events)

ALLES vom Sim — über 80 Felder inklusive `gear_normal_force_n` (X-Plane only — Sim-Limit MSFS), `total_weight_kg`, etc.

### 1.3 Was im 50Hz Sampler-Buffer FEHLT (Datenlücken — werden in v2 gefüllt)

- `gear_normal_force_n` ist im Streamer-Stream vorhanden, aber NICHT im `touchdown_window` Sample-Buffer → **KRITISCH** für X-Plane TD-Validation
- `total_weight_kg` ist im Streamer-Stream vorhanden, aber NICHT im 50Hz-Buffer → benötigt für mass-aware gear_force-threshold + deterministische Acceptance-Replays

### 1.4 Was wir BEWUSST NICHT nutzen werden (addon-unzuverlässig)

Aircraft-Addons reportieren diese Felder unzuverlässig:
- `spoilers_armed` / `spoilers_handle_position`
- `autopilot_*`, `autobrake`
- `engines_running` / Throttle / N1
- Per-gear contact points (`CONTACT POINT IS ON GROUND:0/1/2` in MSFS)
- `total_weight_kg` ist meist ok — bei null/zero Fallback-Pfad nötig (siehe 4.4)

**→ Validation-Logik darf NUR auf zuverlässige Sim-native Daten verlassen.**

### 1.5 Sim-spezifische Datenverfügbarkeit

| Daten | MSFS | X-Plane |
|---|---|---|
| `on_ground` | ✅ konservativ | ⚠️ trigger-happy (Float-edge in 40-50ms möglich) |
| `vs_fpm` | ✅ | ✅ |
| `altitude_agl_ft` | ✅ | ✅ |
| `g_force` | ✅ | ✅ |
| `pitch_deg`, `bank_deg` | ✅ | ✅ |
| `gear_normal_force_n` | ❌ Sim-Limit | ✅ via DataRef |
| `PLANE TOUCHDOWN NORMAL VELOCITY` SimVar (latched) | ⚠️ addon-abhängig | ❌ |
| `total_weight_kg` | ✅ (meist) | ✅ (meist) |

→ **Sim-Trennung ist STRUKTURELL unvermeidbar** weil X-Plane das wichtigste Validation-Signal hat (`gear_normal_force_n`) und MSFS nicht.

---

## 2. Architektur-Übersicht (4 Layer)

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 1: TD-Candidate Detection (sim-spezifisch)            │
│  - sammelt potenzielle TD-Edges aus 50Hz-Stream             │
│  - Multi-Edge-Tracking (kein single-shot)                   │
│  - kein Filter, nur Detection                               │
└────────────────────┬────────────────────────────────────────┘
                     │ candidate stream
┌────────────────────▼────────────────────────────────────────┐
│ Layer 2: TD-Validation (sim-spezifisch)                     │
│  X-Plane: gear_force ist MUST-PASS anchor                   │
│           plus Plausibilitäts-Tests                         │
│  MSFS:    weiches Voting (g_force-spike + AGL + sustained)  │
│  PASS → contact_frame identifiziert (nicht peak-frame!)     │
│  FAIL → markiert als „false-edge", weiter beobachten        │
└────────────────────┬────────────────────────────────────────┘
                     │ contact_frames
┌────────────────────▼────────────────────────────────────────┐
│ Layer 3: VS-Calculation am IMPACT-Frame (sim-agnostic)      │
│  - impact_frame = min vs in [contact-250ms, contact+100ms]  │
│  - load_peak_frame = max gear_force/G (nur Forensik)        │
│  - VS-Cascade mit HARD GUARDS gegen positive Werte          │
│  - Cross-Validation, Confidence-Tag                         │
└────────────────────┬────────────────────────────────────────┘
                     │ touchdown_detected events
┌────────────────────▼────────────────────────────────────────┐
│ Layer 4: LandingEpisode-Aggregation + Final-Selection       │
│  - bündelt false_edges, contact, low_level_touches, settle  │
│  - bei mehreren Episoden: erste = T&G, letzte = final       │
│  - landing_finalized event erst beim Filing                 │
│  - Score = härtester Impact innerhalb final-Episode         │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Layer 1: TD-Candidate Detection

### 3.1 Sim-spezifische Detection

**X-Plane:**
```
candidate_edge wenn:
  prev.in_air == True  AND
  (current.on_ground == True  OR  current.gear_normal_force_n > epsilon)
```

**MSFS:**
```
candidate_edge wenn:
  prev.in_air == True  AND
  current.on_ground == True
```

`prev.in_air` = `!prev.on_ground && (prev.gear_normal_force_n.unwrap_or(0) <= epsilon)`
`epsilon = 1.0 N` (Noise-Floor für gear_force-Sensor)

### 3.2 Was zu speichern ist (pro Candidate)

```rust
struct TdCandidate {
    edge_sample_index: usize,
    edge_at: DateTime<Utc>,  // primäre Zeit-Referenz, nicht sample_count
    edge_agl_ft: f32,
    edge_vs_fpm: f32,
    edge_gear_force_n: Option<f32>,  // X-Plane only
    edge_g_force: f32,
    edge_total_weight_kg: Option<f32>,  // für mass-aware threshold (NEU v2.2)
}
```

### 3.3 Multi-Edge-Tracking

Sampler erfasst **alle** Candidates einer Flight-Session, nicht nur den ersten. `sampler_touchdown_at: Option<DateTime>` wird **abgeschafft** und ersetzt durch `Vec<LandingEpisode>` (siehe Layer 4).

---

## 4. Layer 2: TD-Validation — sim-spezifische Tests

**Grundprinzip:** Schwellwerte werden via `at`-Timestamp gemessen, nicht via Sample-Count. Sampler ist nicht garantiert genau 50 Hz.

### 4.1 X-Plane: gear_force ist MUST-PASS Anchor

**Required (alle 3 müssen PASS):**

| Test | Bedingung |
|---|---|
| **A1: gear_force-impact (MUST)** | `gear_normal_force_n` >= dynamic_threshold (4.4) für mindestens **60ms anhaltend** im Window `[edge_at, edge_at + 500ms]`, mit mindestens 2 distinct samples zur Anti-Glitch-Sicherung |
| **A2: low_agl_persistence** | `agl_ft < 5` für mindestens **1000ms** ab edge_at (Timestamp-basiert) |
| **A3: vs_negative_at_impact** | `vs_at_impact_frame < -10 fpm` (siehe 5.1 für impact_frame Definition) |

**Wenn A1 FAIL → Validation FAIL** (Streifschuss, kein Energie-Transfer).

### 4.2 MSFS: weiches Voting (kein gear_force verfügbar)

**4 Tests, mind. 3 müssen PASS:**

| Test | Bedingung |
|---|---|
| **B1: g_force-spike** | `peak g_force` im Window `[edge_at, edge_at + 500ms]` > 1.05 |
| **B2: sustained_ground_contact** | `on_ground == True` für mindestens **500ms** continuous (Timestamp-basiert) |
| **B3: low_agl_persistence** | `agl_ft < 5` für mindestens **1000ms** ab edge_at |
| **B4: vs_negative_at_impact** | `vs_at_impact_frame < -10 fpm` |

### 4.3 Verifizierte Validierung gegen DAH 3181 (X-Plane Float-Streifschuss)

**Edge 1 (Sample 125, 07:53:58.463):**

| Test | Wert | Result |
|---|---|---|
| A1 (gear_force >= 73550 N für ≥ 60ms) | gear_force = 0 N (Float, Streamer-stream zeigt 0 für die ganze Edge-1-Periode) | **FAIL** |
| A2 (agl<5ft für 1000ms) | agl steigt auf 5.7ft bei 746ms — danach > 5 | **FAIL** |
| A3 (vs negative at impact) | impact_frame würde mit vs ~ +104 berechnet | **FAIL** |

→ **A1 FAIL → MUST-PASS verfehlt → Edge 1 = false-edge** ✓

**Edge 2 (Sample 198, 07:54:02.310):**

| Test | Wert | Result |
|---|---|---|
| A1 (gear_force) | Streamer zeigt 827171 N (= weit über 73550 N threshold) für > 60ms | **PASS** |
| A2 (agl<5ft) | agl bleibt unter 5ft für > 1 sec | **PASS** |
| A3 (vs negative at impact) | impact_frame vs = -414 fpm (siehe 5.1, sample 197) | **PASS** |

→ **3/3 PASS → Edge 2 = validierter contact_frame** ✓

**Edge 3 (Sample 246, 07:54:04.098):**

Bounce-touch 700ms nach Edge 2 mit vs=-49. Wird als `low_level_touch` der gleichen Episode behandelt (Layer 4), nicht als neuer TD.

### 4.4 gear_force-Threshold (mass-aware, zeitbasiert)

```rust
fn gear_force_threshold_n(total_weight_kg: Option<f32>) -> f32 {
    let abs_floor = 1000.0;  // Newton, hartes Minimum
    let mass_ratio = 0.03;   // = 3% des statischen Gewichts
    let dynamic = total_weight_kg
        .filter(|w| *w > 100.0)  // Plausibilität
        .map(|w| w * 9.80665 * mass_ratio)
        .unwrap_or(abs_floor);
    dynamic.max(abs_floor)
}
```

**Beispiele:**
| Aircraft | total_weight_kg | dynamic | final threshold |
|---|---|---|---|
| Cessna 152 | 757 | 222 N | **1000 N** (floor wins) |
| A320 | 73000 | 21478 N | **21478 N** |
| A330 (DAH 3181) | 250000 | 73550 N | **73550 N** |
| B747 | 333400 | 98099 N | **98099 N** |
| Sim ohne weight | None | — | **1000 N** (floor) |

**Confirmation-Window (zeitbasiert):**

```
A1 PASS wenn:
  EXISTS Zeit-Intervall T = [t_start, t_end] innerhalb [edge_at, edge_at + 500ms]
    sodass:
      - t_end - t_start >= 60ms
      - alle Samples in T haben gear_force_n >= threshold
      - T enthält mindestens 2 distinct samples (Anti-Glitch)
```

DAH 3181: Streamer-stream zeigt gear_force von 0 → 827kN → 1245kN → 1406kN ... — sustained über 60ms × > 2 Samples → PASS.

### 4.5 Verifizierte Validierung gegen MSFS-Flüge (alle 4 mit landing_analysis)

Beispiel CFG 785:

| Test | Wert | Result |
|---|---|---|
| B1 (g_force-spike) | peak g_force = 1.18 | PASS |
| B2 (sustained 500ms) | on_ground bleibt True (rollout) | PASS |
| B3 (agl<5ft 1000ms) | bleibt unter 1ft | PASS |
| B4 (vs negative at impact) | vs_at_impact_frame ≈ -142 | PASS |

→ **4/4 PASS → validierter contact_frame → vs_at_impact_frame = -142** (= unverändert zu heute)

---

## 5. Layer 3: VS-Calculation am IMPACT-Frame

### 5.1 Frames im Window nach contact (klare Trennung)

```
contact_frame:        erste Force-Threshold-Überschreitung (X-Plane)
                      ODER  erste on_ground=True die A1 (oder B-Voting) bestanden hat
impact_frame:         min(vs_fpm) im Zeit-Window [contact_at - 250ms, contact_at + 100ms]
                      → das ist die echte „raw härteste Sink-Rate beim Aufsetzen"
initial_load_peak:    max(gear_force_n bei X-Plane, g_force bei MSFS) im engen Window
                      [contact_at, contact_at + 500ms]
                      → initial impact strength (= Energie-Übertragung beim Aufprall)
episode_load_peak:    max(gear_force_n bei X-Plane, g_force bei MSFS) ueber die GANZE
                      Episode (incl. rollout, brake-application)
                      → nur Forensik. Bei DAH 3181 = 1635 kN bei 07:54:08
                        (= 5.7 sec NACH contact, im rollout)
```

**Score-Klassifizierung** nutzt `peak_g` aus `initial_load_peak` (= initial impact),
nicht episode_load_peak (= mass-settle / brake-application).

**Klarstellung Frame-Semantik (DAH 3181 als Beispiel):**

| Frame | Sample-Index | VS | Bedeutung |
|---|---|---|---|
| contact_frame | 198 | -401 fpm | erste on_ground=True die Validation passed |
| **impact_frame** | **197** | **-414 fpm** | **min vs in window — was als Landing-Rate gilt** |
| load_peak_frame | (späterer Sample, evtl ausserhalb 50Hz-buffer) | — | nur Forensik |
| load-transfer | 200 | -334 fpm | „post-impact dampening" — NICHT die Landing-Rate |

→ **Was als Landing-Rate gilt = vs am impact_frame** = -414 fpm bei DAH 3181.

### 5.2 VS-Berechnung am impact_frame (sim-agnostic Cascade)

```rust
fn compute_landing_vs(
    contact_at: DateTime<Utc>,
    samples: &[Sample],
) -> Result<LandingRateResult, RejectionReason> {
    // impact_frame = min vs im Time-Window
    // NaN-sicher: nur finite vs_fpm Samples + total_cmp als fallback
    let impact_frame = samples.iter()
        .filter(|s| {
            let dt_ms = (s.at - contact_at).num_milliseconds();
            dt_ms >= -250 && dt_ms <= 100 && s.vs_fpm.is_finite()
        })
        .min_by(|a, b| a.vs_fpm.total_cmp(&b.vs_fpm))
        .ok_or(RejectionReason::EmptyWindow)?;

    let vs_at_impact = impact_frame.vs_fpm;
    
    // Smoothed Werte AM IMPACT-FRAME
    let vs_smoothed_500_at_impact = avg_in_time_window(samples, impact_frame.at, -500..0);
    let vs_smoothed_1000_at_impact = avg_in_time_window(samples, impact_frame.at, -1000..0);
    let pre_flare_peak = min_in_time_window(samples, impact_frame.at, -3000..-500);
    
    let chosen = if vs_at_impact < -10.0 {
        (vs_at_impact, "vs_at_impact_frame", Confidence::High)
    } else if vs_smoothed_500_at_impact < -10.0 {
        (vs_smoothed_500_at_impact, "vs_smoothed_500ms_at_impact", Confidence::Medium)
    } else if vs_smoothed_1000_at_impact < -10.0 {
        (vs_smoothed_1000_at_impact, "vs_smoothed_1000ms_at_impact", Confidence::Low)
    } else if pre_flare_peak < 0.0 {
        (pre_flare_peak, "pre_flare_peak", Confidence::VeryLow)
    } else {
        return Err(RejectionReason::AllSourcesPositive);  // HARD REJECT
    };
    
    finalize_vs(chosen.0)?;
    
    Ok(LandingRateResult {
        vs_fpm: chosen.0,
        source: chosen.1,
        confidence: chosen.2,
        contact_frame_at: contact_at,
        impact_frame_at: impact_frame.at,
        ...
    })
}
```

### 5.3 HARD GUARDS (strukturell)

```rust
fn finalize_vs(candidate_fpm: f32) -> Result<f32, RejectionReason> {
    if candidate_fpm > 0.0 {
        return Err(RejectionReason::PositiveVs);
    }
    if candidate_fpm < -3000.0 {
        return Err(RejectionReason::ImplausiblyHigh);
    }
    Ok(candidate_fpm)
}
```

**Bei `Err(...)`:** Kein Score finalisiert, Cockpit zeigt Banner „Touchdown forensics inconclusive — please review manually".

### 5.4 Verifizierte Berechnung gegen DAH 3181

```
contact_at = 07:54:02.310 (sample 198, on_ground=True, gear_force passed A1)
Window für impact_frame: [07:54:02.060, 07:54:02.410]

VS-Werte im Window:
  sample 193  07:54:02.060  vs=-371
  sample 194  07:54:02.106  vs=-391
  sample 195  07:54:02.166  vs=-406
  sample 196  07:54:02.238  vs=-413
  sample 197  07:54:02.276  vs=-414  ← MIN VS
  sample 198  07:54:02.310  vs=-401  (contact_frame)
  sample 199  07:54:02.359  vs=-401
  sample 200  07:54:02.400  vs=-334  (load-transfer)

→ impact_frame = sample 197, vs_at_impact = -414 fpm
→ Confidence = High (vs_at_impact < -10 ✓, alle Cross-Quellen konsistent negativ)
→ Score-Bucket: -400 ≥ vs > -600 → **firm**
```

---

## 6. Layer 4: LandingEpisode

### 6.1 Datenmodell

```rust
struct LandingEpisode {
    episode_index: u8,
    
    /// Snapshot des Aircraft-Zustands beim contact (für Replay deterministisch)
    aircraft_state_at_contact: AircraftStateSnapshot,
    
    /// false-edges die VOR diesem contact_frame zur Episode gehören
    /// (Float-Streifschüsse die A1 nicht bestanden)
    false_edges: Vec<FalseEdge>,
    
    /// echter erster Bodenkontakt (validiert)
    contact: ContactDetail,
    
    /// nachfolgende low-level Touches innerhalb derselben Episode
    /// (= aircraft bleibt unter 50ft AGL, kein climb-out > 100ft)
    /// Beispiel PTO 705: Touch 1 (-182), Pause 2.3s, Touch 2 (-61)
    low_level_touches: Vec<LowLevelTouch>,
    
    /// finaler Settle-Frame (Räder bleiben auf, gs sinkt)
    settle: Option<SettleDetail>,
    
    /// load-peak (Forensik)
    load_peak: LoadPeakDetail,
    
    /// härtester Impact (kann contact ODER low_level_touch sein)
    /// = der VS der für das Scoring zählt
    hardest_impact_vs_fpm: f32,
    hardest_impact_source: HardestImpactSource,  // Contact | LowLevelTouch(idx)
    
    classification: EpisodeClass,  // FinalLanding | TouchAndGo | GoAround | Pending
}

struct AircraftStateSnapshot {
    total_weight_kg: Option<f32>,  // wichtig für mass-aware threshold (Replay)
    fuel_total_kg: Option<f32>,
    aircraft_icao: Option<String>,
    sim: SimKind,  // X-Plane oder MSFS
}

enum EpisodeClass {
    FinalLanding,   // aircraft blieb am Boden, gs sinkt — Pilot ist gelandet
    TouchAndGo,     // aircraft hob nach Touch wieder ab, stieg auf < 1000ft AGL,
                    // kam danach wieder runter (Pattern-Flug)
    GoAround,       // aircraft stieg > 1000ft AGL nach dem Touch
    Pending,        // noch nicht klassifiziert (Episode läuft noch)
}
```

### 6.2 „Final Landing" Episode-Finalisierung (zum PIREP-Filing-Zeitpunkt)

Eine Episode wird `EpisodeClass::FinalLanding` wenn:
- aircraft bleibt für mindestens 30 sec UNTER 50ft AGL nach contact
- UND groundspeed sinkt UNTER 30kt
- UND keine climbout-Sequenz > 100ft AGL nach contact

**Beim PIREP-Filing** wird die Episode mit `classification == FinalLanding` als „die Landung" gewählt. Wenn mehrere FinalLanding existieren → nimm letzte.

### 6.3 Beispiel PTO 705 (Touch-and-Go mit 2 low-level touches)

**Verifiziert aus JSONL:**

```
Episode 0:
  aircraft_state_at_contact: { total_weight=..., sim=Msfs2024 }
  false_edges: []  (erster on_ground edge war direkt validated)
  contact: 07:54:30.020
    - vs_at_impact = -182 fpm
    - sustained ground 307ms (= UNTER B2-Threshold von 500ms!)
    - validation:
        B1 (g_force-spike) PASS    (peak g = 1.254 > 1.05)
        B2 (sustained >=500ms) FAIL (307ms)
        B3 (agl<5ft >=1000ms) PASS
        B4 (vs negative at impact) PASS  (-182 < -10)
        → 3/4 PASS → VALIDATED via Voting-Modell
    - confidence: Medium (B2 fail = nicht-sustained, deutet auf
      T&G/Bounce hin, was ja auch der Fall ist)
  low_level_touches: [
    { at: 07:54:32.631, vs_at_impact = -61 fpm, sustained 339ms, agl_max=2.97 }
    // (Pilot hat zweiten leichten Touch im Float bevor Climb-out)
  ]
  settle: None  (aircraft hob danach wieder ab)
  initial_load_peak (im 500ms window): siehe Forensik
  episode_load_peak: max(gear_force) ueber ganze Episode (X-Plane only)
  hardest_impact_vs_fpm: -182  (contact härter als low_level_touch)
  classification: TouchAndGo  (climb-out auf 1560ft AGL nach 30s+)

Episode 1:
  contact: 08:01:29.820, vs ≈ -111 fpm, sustained > 30sec
  low_level_touches: []
  settle: 08:01:42, gs<30kt
  classification: FinalLanding

→ PIREP-Score: vom Episode 1 (~ -111 fpm)
→ PIREP-Notes: „Touch-and-Go detected (Episode 0, 2 low-level touches:
                vs=-182, vs=-61). Final landing Episode 1 (vs ≈ -111)."
```

### 6.4 Beispiel DAH 3181 (Float false-edge + echter TD + Bounce)

```
Episode 0:
  aircraft_state_at_contact: { total_weight=250000, sim=XPlane12 }
  false_edges: [
    Edge 1 @ 07:53:58.463
      - 44ms ground contact (sample 125-126)
      - gear_force = 0 N (FAIL A1)
      - reason: "gear_force_below_threshold"
  ]
  contact: 07:54:02.310 (sample 198)
    - vs_at_impact = -414 fpm (sample 197 = min vs in window)
    - gear_force-peak = 827171 N (PASS A1, threshold 73550 N)
    - confidence: High
  low_level_touches: [
    { at: 07:54:04.098, vs_at_impact ≈ -49 fpm, sustained 200ms+ }
  ]
  settle: 07:54:30+, rollout
  initial_load_peak (im 500ms window nach contact):
    ~ 800-1200 kN (geschätzt — Streamer-stream nur 3s cadence,
    erster Sample mit gear_force = 827171 N bei 07:54:02.402)
  episode_load_peak: 1635171 N @ 07:54:08
    (= 5.7 sec NACH contact, im rollout — Forensik-only)
  hardest_impact_vs_fpm: -414  (contact härter als low_level_touch)
  classification: FinalLanding

→ PIREP-Score: vs = -414 fpm, score = firm
→ Confidence: High
```

### 6.5 Bounce-Score / Härtester-Impact-Regel

`hardest_impact_vs_fpm = min(contact.vs_at_impact, low_level_touches.iter().map(|t| t.vs_at_impact))`

→ Beispiel Hard-Bounce: contact = -600 fpm, dann low_level_touch = -200 fpm → **PIREP-Score = -600** (= härtester Impact). Bounce wird als Penalty/Note dokumentiert.

### 6.6 Während des Flugs: Cockpit-UX

- Per validated TD wird `touchdown_detected` event emittiert
- Cockpit zeigt **vorläufigen** Score („preliminary, episode N")
- Bei Climb-out > 100ft AGL > 30s: Banner „last touch was T&G/Go-Around, waiting for final"
- Beim PIREP-Filing: `landing_finalized` event mit `final_episode_index`

---

## 7. Schema-Änderungen

### 7.1 `TouchdownWindowSample` (50Hz Sampler-Buffer)

```rust
pub struct TouchdownWindowSample {
    // bestehende Felder ...
    
    // NEU v2.2:
    pub gear_normal_force_n: Option<f32>,  // X-Plane Some, MSFS None
    pub total_weight_kg: Option<f32>,      // für mass-aware threshold + Replay-Determinismus
}
```

**Backward-Compat:** alte JSONLs ohne die Felder deserialisieren mit `None`.

### 7.2 Event-Naming + Backward-Compat-Bridge (NEU v2.2)

| Alt (v0.6.x) | Neu (v0.7.0) | Backward-Compat |
|---|---|---|
| `touchdown_complete` (mit voreiligem `is_final`) | `touchdown_detected` (pro contact_frame) | — |
| — | `landing_finalized` (am PIREP-Filing, mit `final_episode_index`) | wird zusätzlich als legacy `touchdown_complete` event mit `is_final=true` gespiegelt für 1-2 Releases |

**Übergangsphase:** Recorder/Web können auf altem Stand bleiben — der Client emittiert beide Events parallel:
- `touchdown_detected` (neu, mit `forensics_version: 2`)
- `touchdown_complete` (legacy-bridge, mit `is_final=true` aus `landing_finalized` abgeleitet)

Nach 1-2 Releases (= aeroacars-live Recorder updated) wird die legacy-bridge entfernt (v0.8.0).

### 7.3 `forensics_version` Feld (Cutoff via Version)

In allen TD-relevanten Events + im PIREP-payload:

```rust
struct TouchdownDetectedEvent {
    forensics_version: u8,  // = 2 ab v0.7.0
    episode_index: u8,
    contact_frame_at: DateTime<Utc>,
    impact_frame_at: DateTime<Utc>,
    landing_rate: LandingRateResult,
    aircraft_state_at_contact: AircraftStateSnapshot,
    // ...
}
```

---

## 8. Stop-Bedingungen / Edge-Cases

### 8.1 Kein TD detected nach 30s in Phase=Landing

→ Kein automatischer Score. Cockpit zeigt Banner:
> „Touchdown forensics inconclusive — please review and file PIREP manually if landing was successful."

JSONL hat trotzdem alle Samples für spätere Forensik.

### 8.2 Pilot quittete App vor finalem TD

→ Beim Resume: `Vec<LandingEpisode>` wird aus persistierter active_flight.json restored. Sampler beobachtet weiter.

### 8.3 PIREP-Filing OHNE jegliche validierte Episode

→ `landing_finalized` Event mit `final_episode_index: None`, Score-Felder bleiben null. Banner wie 8.1.

### 8.4 SimVar `PLANE TOUCHDOWN NORMAL VELOCITY` (MSFS) als Confidence-Boost

- Wenn gesetzt UND `|simvar_vs - vs_at_impact| < 50 fpm` → `Confidence::High` (auch wenn primary nur Medium war)
- Wenn divergent (> 50 fpm Spread) → log warning mit beiden Werten, Score bleibt vom impact_frame
- **Niemals** SimVar als primary

---

## 9. Migration & Rollout

### 9.1 Cutoff via Version

- Alle Events ab v0.7.0 tragen `forensics_version: 2`
- Recorder akzeptiert beide (v1 + v2), wertet via Version aus
- Datum (10.05.26 02:44) als Rollout-Hinweis im UI

### 9.2 Schema-Backward-Compat

- TouchdownWindowSample.gear_normal_force_n + total_weight_kg optional → alte JSONLs deserialisieren weiter
- touchdown_complete legacy-bridge (siehe 7.2) für 1-2 Releases

### 9.3 Release-Pfad

- v0.7.0 als Major-Bump (semantische Touchdown-Score-Änderung)
- Pilot-Schutz: erst Prerelease, dann Test-Flight, dann Latest
- Bilingual Notes mit Vorher/Nachher Beispielen
- Discord-Ankündigung mit explizitem Hinweis: „Score-Logik fundamental anders, vor allem bei X-Plane Float-Landings und Touch-and-Go"

---

## 10. Akzeptanz-Tests

**Toleranz:** `±5 fpm`, gleicher Score-Bucket, gleicher Episode-Count, gleiche Klassifizierung — nicht bit-identisch.

**Score-Buckets:**
- smooth: `0 > vs > -200`
- acceptable: `-200 ≥ vs > -400`
- firm: `-400 ≥ vs > -600`
- hard: `vs ≤ -600`

| Flug | Sim | Erwartung neue Logik | Heute | Toleranz |
|---|---|---|---|---|
| PTO 105 GA | MSFS | vs ∈ [-60, -50] fpm, score=smooth, 1 Episode FinalLanding | -55/100 | ±5 fpm, smooth |
| **PTO 705 T&G** | MSFS | **2 Episoden**: Ep 0 = TouchAndGo (contact=-182, low_level_touches=[vs=-61], hardest=-182); Ep 1 = FinalLanding mit eigenem Score (~ -111) | -182 vom Streifschuss ❌ | bestehen wenn 2 Episoden + Final = Ep 1 + Ep 0 enthält 1+ low_level_touch |
| DLH 304 | MSFS | vs ∈ [-362, -352] fpm, score=acceptable, 1 Episode | -357/80 | ±5 fpm |
| CFG 785 | MSFS | vs ∈ [-147, -137] fpm, score=smooth, 1 Episode | -142/100 | ±5 fpm |
| DLH 742 | MSFS | vs ∈ [-196, -186] fpm, score=smooth, 1 Episode | -191/100 | ±5 fpm |
| **DAH 3181** | **X-Plane** | **vs am impact_frame ∈ [-415, -395] fpm, score=firm, 1 Episode FinalLanding mit false_edges=1 + low_level_touches=1, hardest_impact_source=Contact** | +104/80 ❌ | bestehen wenn vs ≤ -395 + Float als false_edge erkannt + bounce als low_level_touch |

→ **DAH 3181 Score-Korrektur:** -414 fpm = **firm** Bucket. Score-Label könnte zusätzlich „firm landing with float-streifschuss + bounce" als Detail-Note enthalten.

---

## 11. Implementation-Plan (mit Zeit-Schätzung)

| Phase | Was | Zeit |
|---|---|---|
| A | Sample-Schema erweitern (`gear_normal_force_n` + `total_weight_kg`) + serde-default | 30 min |
| B | TD-Candidate-Detection (Layer 1) — sim-spezifisch + multi-edge-tracking | 1.5 h |
| C | TD-Validation (Layer 2) — A-Tests X-Plane (zeitbasiert!) + B-Tests MSFS | 2 h |
| D | impact_frame / contact_frame / load_peak_frame Berechnung (Layer 3) | 1 h |
| E | VS-Cascade + HARD GUARDS + Cross-Validation | 1.5 h |
| F | LandingEpisode Datenmodell + AircraftStateSnapshot + Aggregation | 2 h |
| G | Final-Landing-Selection + Bounce-Score + Episode-Klassifizierung | 1.5 h |
| H | Sampler-Refactor (Multi-Edge, Episoden-State-Machine) | 2.5 h |
| I | Event-Renaming (touchdown_detected + landing_finalized + legacy-bridge) | 1.5 h |
| J | forensics_version Marker in Events + PIREP-payload | 30 min |
| K | Frontend (Cockpit-Banner + Confidence-Badge + Episode-Anzeige) | 2 h |
| L | Acceptance-Tests gegen die 6 JSONLs | 1.5 h |
| M | Bilingual Release-Notes + Discord-Ankündigung | 30 min |
| N | Build + Deploy + Pilot-Test | 1 h |

**Gesamt:** ~19 Stunden

---

## 12. Was BEWUSST NICHT in v0.7.0 ist

- **Re-Score alter PIREPs** — Forward-only
- **Per-gear contact points** — addon-unzuverlässig
- **Throttle/N1/Spoilers/Autobrake** — addon-unzuverlässig
- **Synthetic-TD Auto-Score** (Sektion 8.1)
- **Re-Score-Tool für Pre-v0.7.0 PIREPs** — könnte v0.7.1 sein

---

## 13. Risiken

| Risiko | Mitigation |
|---|---|
| gear_force-Threshold zu strict → echte leichte TDs werden false-edge | abs_floor=1000N als hartes Minimum, mass_ratio dynamisch nur darüber |
| gear_force-Threshold zu loose → Streifschuss als TD durch | Confirmation 60ms × 2+ samples zeitbasiert |
| MSFS-Voting zu loose ohne gear_force | Cross-Validation Spread liefert Confidence-Hinweis bei divergenten Quellen |
| impact_frame-Window zu eng → echter sink-min außerhalb | [-250ms, +100ms] empirisch gewählt aus DAH 3181 + 4 MSFS-Flügen, kann nachkalibriert werden |
| Episode-Klassifizierung falsch (T&G vs Bounce) | Schwellwert 100ft AGL klar dokumentiert |
| Schema-Änderung breaks alte JSONLs | Optional fields, serde-default, Acceptance-Test gegen pre-v0.7.0 JSONLs |
| Recorder/Web nicht synchron deployt | Legacy-bridge `touchdown_complete` aus `landing_finalized` für 1-2 Releases (7.2) |
| Pilot verwirrt durch Multi-Episode-Anzeige | UI: nur Final-Episode-Score prominent, andere als kollabierte Sekundär-Info |

---

## 14. Open Questions — beantwortet

1. **Bounce-Score:** ✅ härtester Impact innerhalb der Episode (6.5)
2. **Final Landing Definition:** ✅ Episode-Finalisierung via 30sec/50ft/30kt + keine climbout (6.2)
3. **Synthetic-TD Fallback:** ✅ kein Auto-Score, nur Review-Banner (8.1)
4. **gear_force-Threshold:** ✅ mass-aware mit absolute floor + zeitbasierte Confirmation (4.4)
5. **MSFS SimVar Cross-Check:** ✅ Confidence-Boost bei Plausibilität, Warnung bei Divergenz (8.4)

---

## 15. Bekannte Bugs die durch diese Spec gefixt werden

| # | Bug | Wie gefixt |
|---|---|---|
| 1 | vs_at_edge unconditional override → positive Landerate | HARD GUARD (5.3) + Cascade auf impact_frame (5.1) |
| 2 | vs_estimate_xp/msfs nicht negative_only | Cascade hat `< -10` filter (5.2) |
| 3 | Sampler is_none()-Guard verhindert zweiten TD | Multi-Edge-Tracking + LandingEpisodes (3.3, 6.1) |
| 4 | touchdown_complete fehlt beim zweiten Touch | Pro contact_frame ein `touchdown_detected` event (7.2) |
| 5 | landing_estimate_window_ms unvollständig gesetzt | Wird im neuen Schema sauber gesetzt |
| 6 | bounce_count Inkonsistenz analysis vs scored | Bounce als Teil der Episode (6.1), single source of truth |
| 7 | flare_detected Heuristik unzuverlässig | Wird nicht mehr für Selection genutzt — nur informativ |
| 8 | X-Plane on_ground edge-trigger-happy bei Float | A1 (gear_force) ist MUST-PASS Anchor (4.1) — strukturell zu |
| 9 | T&G/Go-Around: erster Streifschuss als Score | Episode-Aggregation + classification (6.2, 6.3) |

---

## Anhang A — v2.0 → v2.1 Diff (zur Referenz)

[13 Punkte aus erstem Review — siehe v2.1 Changelog Tabelle in vorherigen Versionen des Dokuments]

| Komponente | v2.0 | v2.1 |
|---|---|---|
| Validation-Modell | „3 von 4 Tests" gleichberechtigt | X-Plane: gear_force MUST-PASS; MSFS: weiches 3-of-4 |
| TD-Frame | „peak gear_force frame" | 3 separate Frames: contact / impact / load_peak |
| VS-Quelle | vs_at_edge | vs_at_impact_frame |
| Schwellwerte | sample-count basiert | timestamp-basiert |
| gear_force-Threshold | „> 0 für 200ms" | aircraft-mass-aware (1000N floor + 0.03 × static weight) |
| Datenmodell | Vec<ValidatedTd> | strukturierte LandingEpisode |
| Event-Naming | touchdown_complete | touchdown_detected + landing_finalized |
| Cutoff | nur Datum | forensics_version + Datum als Rollout-Hinweis |
| Acceptance | bit-identisch | ±5 fpm, gleicher Score-Bucket |
| Synthetic-Fallback | als VeryLow Score erlaubt | nur Review-Banner |
| MSFS-SimVar | optional cross-check | Confidence-Boost mit Divergenz-Warnung |
| Bounce-Score | „letzter sustained TD" | härtester Impact innerhalb Episode |
| DAH 3181 Erwartung | „smooth" | acceptable (war noch falsch in v2.1, in v2.2 korrigiert auf firm) |

---

**Ende Spec v2.3.** Status: **Approved for v0.7.0 implementation** (VA-Owner sign-off). Implementation startet nach Sektion 11, Reihenfolge: Backend (A-J) → Replay-Acceptance (L) → Frontend (K) → Release (M, N).
