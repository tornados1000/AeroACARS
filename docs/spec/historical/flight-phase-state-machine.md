# Flight-Phase State-Machine — QS-Inventur fuer Bug-Untersuchung

**Status:** v1.5 — **Draft for QS Review** (VPS-Daten-Analyse + 2 reale Regression-Kandidaten)
**Zweck:** Vollstaendige Inventur aller Phase-Wechsel + Trigger + Side-Effects + Anti-Flicker-Mechaniken. Damit kann VA-Owner / QS systematisch durchgehen und potenzielle Bug-Klassen finden bevor sie als Live-Bug auftauchen.
**KEIN Implementierungs-Auftrag** — diese Spec dokumentiert NUR den Status-Quo + markiert Verdachtsstellen.

> **Anker-Konvention (seit v1.1):** Diese Spec referenziert Code via **Funktions- und Konstanten-Namen**, nicht via Zeilennummern (driften zu schnell). Wo Zeilennummern stehen, sind sie als "Stand v0.7.4" markiert und nur als Suchhilfe.

---

## 0. Warum dieses Dokument

Die Phase-State-Machine in `step_flight()` (~600 Zeilen in `lib.rs`) ist ueber Monate gewachsen und hat zwischenzeitlich mehrere Live-Bugs produziert (PMDG-B738 53819ft AGL-Glitch, GSX-Repositioning-Trigger, MSFS-Pause-Race etc.). Jede Korrektur hat eine Anti-Flicker-Schutzschicht hinzugefuegt — aber niemand hat den Gesamt-Zustand systematisch dokumentiert.

Diese Spec ist die Antwort. Pro Transition: was triggert sie, welche Schwellen, welche Anti-Flicker-Mechaniken sind aktiv, welche Side-Effects passieren. Plus eine Verdachts-Liste (markiert mit **[VERDACHT]**) mit Stellen die im Code-Audit verdaechtig wirkten.

### 0.1 Changelog v1.4 → v1.5 (VPS-Daten-Analyse)

VA-Owner hat 29 echte Pilot-PIREP-JSONLs + DB-Snapshot vom VPS untersucht. Ergebnis: **2 Verdachts-Punkte sind historisch belegt**, plus eine neue Bug-Klasse + konkrete Daten-Coverage-Tabelle. Reine Spec-Erweiterung, kein Code-Fix.

| # | Aenderung |
|---|---|
| **§15** (NEU) | VPS-Daten-Coverage-Tabelle: was DB vs JSONL liefert, was real fuer welches Szenario nutzbar ist |
| **§16** (NEU) | Reale Regression-Kandidaten: URO913 (Arrived-Fallback), DLH742 (echtes Holding), PTO105 (Holding-Leak), PTO705 (Go-Around) |
| **§13.1 → §13.5** alle Verdachts-Klassen mit "real belegt"-Markierung wenn aus VPS-Daten bestaetigt |
| **§13.9** (NEU) | Holding-Pending leakt phasenuebergreifend — PTO105 zeigt 5.2s Holding statt 90s Dwell. `holding_pending_since` wird bei Approach→Final/Climb-Wechseln nicht zurueckgesetzt |
| **§14** S9-S12 in Status "synthetisch noetig" markiert (kein VPS-Daten-Beleg) |
| **§14.2** (NEU) Test-Strategie: Real-Replay vs synthetisch vs manuell pro Szenario |

---

### 0.2 Changelog v1.3 → v1.4 (Round-4 Doku-Glaettung)

2 P2 + 2 P3 aus Review-Round 4. Reine Doku-Konsistenz, keine neuen Themen.

| # | Fix |
|---|---|
| **P2.1** | §3.1 Schreibstellen-Aufzaehlung praeziser: **2 direkte Writes im Final→Landing-Pfad (lib.rs:11679, 11734) + 2 guarded Rescue-Writes im Arrived-Fallback (lib.rs:12767, 12802 — beide im selben `if landing_at.is_none() && takeoff_at.is_some()` Block)**. v1.3 sagte "3 unconditional + 1 guarded" was 12767 falsch zaehlte |
| **P2.2** | §13.1 Verdachts-Formulierung an Authority-Section angeglichen: Sampler schreibt `sampler_touchdown_at`, Streamer kopiert nach `landing_at`. Race ist NICHT "beide schreiben landing_at" sondern "Streamer setzt landing_at bevor Sampler sampler_touchdown_at validiert hat" |
| **P3.1** | §10 Holding-Exit: kann zur vorherigen Phase zurueck (Cruise ODER Approach), nicht nur Approach |
| **P3.2** | §10 Landing→TaxiIn Score-Klassifikation: Score wird im Landing-Window finalisiert (kann vor/um TaxiIn herum passieren), nicht hart an die Phase-Transition gebunden |

---

### 0.3 Changelog v1.2 → v1.3 (Round-3 Doku-Glaettung)

2 P1 + 3 P2 + 1 P3 aus VA-Owner Review-Round 3. Reine Doku-Konsistenz-Korrekturen, kein neues Thema.

| # | Fix |
|---|---|
| **P1.1** | §3.1 sagte "alle 4 Sites idempotent" — falsch. Final→Landing-Pfad (lib.rs:11679) setzt `landing_at = Some(actual_td_at)` UNCONDITIONAL, danach (lib.rs:11734) wird `landing_at = Some(sampler_at)` ueberschrieben wenn `sampler_touchdown_at` vorhanden. Nur lib.rs:12802 ist guarded. v1.3: §3.1 ehrlich beschrieben |
| **P1.2** | §10 Side-Effects-Tabelle hatte alten "via finalize_landing_rate" Text — widerspricht §3/§6 v1.2-Korrektur. Aktualisiert |
| **P2.1** | §14-Header sagte "10 Szenarien" obwohl S11-S13 schon drin → "13 Szenarien" |
| **P2.2** | §14.1 Replay-Test-Empfehlung erweitert um S11 + S12 (sind die kritischen neuen Risiken) |
| **P2.3** | S12-Erwartung war zu optimistisch ("Resume erfordert User-Bestaetigung — Bug geloest"). Neu: "Vor Bestaetigung keine Transition; nach Bestaetigung muss Sanity-Tick verhindern dass erster Snapshot Phantom-Landing ausloest" |
| **P3** | §0 Anker-Konvention sagte "v1.1" → "seit v1.1" |

---

### 0.4 Changelog v1.1 → v1.2 (Round-2 Korrekturen)

3 P1 + 4 P2 sachliche Fehler aus Review-Round 2. v1.1 beschrieb teils ein "Idealmodell" das nicht zur Code-Realitaet passt — v1.2 zieht das gerade ohne neue Themen aufzumachen.

| # | Fix |
|---|---|
| **P1.1** | `landing_at` Authority falsch beschrieben. `finalize_landing_rate()` setzt nur `landing_rate_fpm/peak_vs/confidence/source`, **NICHT** `landing_at`. Sampler setzt `sampler_touchdown_at`, der Streamer-Tick kopiert spaeter nach `landing_at` (lib.rs:11679, 11735, 12767, 12802) |
| **P1.2** | Final → Landing wird vom **Streamer-Snapshot** getriggert (`!was_on_ground && snap.on_ground`), NICHT vom Sampler. Sampler liefert nur den besseren Timestamp/Rate-Wert, setzt aber keine Phase. Wichtig fuer §3.1 Race-Frage |
| **P1.3** | Universal Arrived-Fallback-Bedingung praezisiert: Code prueft nur `snap.on_ground && snap.engines_running == 0` (lib.rs:12680), NICHT groundspeed_kt < 1. "Stationary" war falsch — der 30s-Dwell misst tatsaechlich "on ground + engines off", nicht echten Stillstand |
| **P2.1** | Phase-Enum hat **17** Varianten (inkl. Holding), nicht 16 |
| **P2.2** | Phase-Diagramm korrigiert: Holding nur `Cruise ↔ Holding` und `Approach ↔ Holding`, nicht aus Climb |
| **P2.3** | Boarding-Transitions nutzen **`on_surface`** (= `on_ground OR (agl < 5ft && \|VS\| < 50fpm)`) — wichtig fuer Seaplanes |
| **P2.4** | Invariante I1 (`takeoff_at` "genau einmal") relativiert: Code kann Takeoff in mehreren Sonderpfaden setzen (Boarding direct, TaxiOut direct, TakeoffRoll edge). Korrekte Invariante: "nach erstem Takeoff nicht durch T&G ueberschreiben" |

Plus 4 QS-Test-Verfeinerungen in §14.

---

### 0.5 Changelog v1.0 → v1.1

VA-Owner Review-Round 1 hat 3 P1 + 3 P2 sachliche Fehler aufgedeckt — alle korrigiert:

| # | Fix |
|---|---|
| **P1.1** | Holding ist real implementiert (`check_holding_entry` + Transitionen Cruise→Holding und Approach→Holding mit Exit-Pfad). v1.0 sagte faelschlich "nicht implementiert" |
| **P1.2** | Go-Around-Schwellen korrigiert: `GO_AROUND_AGL_RECOVERY_FT = 150` (nicht 200), `GO_AROUND_MIN_VS_FPM = 300` (nicht 500) |
| **P1.3** | Climb → Descent hat 3 Zweige (standard_tod / low_altitude_descent / catchall) mit `lost_from_peak > 200ft`-Schutz, nicht nur `VS < -500fpm` |
| **P2.1** | phpVMS-Mapping: Preflight→BST (nicht INI). Cruise/Descent/Holding alle ENR (kein dedizierter Descent-Code) |
| **P2.2** | Pause/Slew-Freeze ist im Code (`if snap.paused \|\| snap.slew_mode { return None }`). Echter Verdacht ist erster Tick NACH Resume, nicht der Slew selbst |
| **P2.3** | Code-Anker auf Funktionsnamen umgestellt, Zeilennummern nur noch als "Stand v0.7.4" Hinweis |

Plus Authority-Model + Critical-Invariants + Soft/Hard-Phases + 10-Szenarien-Testmatrix als neue Sektionen aus der QS-Diskussion.

---

## 1. Phase-Enum (sim-core)

`crates/sim-core/src/lib.rs::FlightPhase` — **17 Varianten** (inkl. Holding):

```
Preflight → Boarding → Pushback → TaxiOut → TakeoffRoll → Takeoff
   → Climb → Cruise (↔ Holding) → Descent
   → Approach (↔ Holding) → Final → Landing
   → TaxiIn → BlocksOn → Arrived → PirepSubmitted
```

`Holding` ist eine echte Phase mit Detection in `check_holding_entry()`. **Eintrittspfade nur** aus `Cruise` und `Approach` (NICHT aus Climb). Exit-Pfad: zurueck zur vorherigen Phase (Cruise oder Approach) oder weiter zu Approach falls echter Descent waehrend Hold erkannt wird.

---

## 2. Hauptfunktion `step_flight`

`lib.rs::step_flight()` (~Stand v0.7.4 lib.rs:10910) — wird vom Streamer-Tick (5-30 s je nach Phase) aufgerufen. Reihenfolge in einem Tick:

1. Anti-Flicker-State refreshen (Engines, Pushback)
2. Distance-Accounting (`distance_nm` += Haversine — siehe **§9.6** zu Holding-Distanz)
3. Position-Counter, last_lat/lon, fuel-Tracking
4. Block-Fuel-Peak-Tracker (mit Defuel-Erkennung > 200 kg sudden drop)
5. Peak-Altitude-Tracker
6. **`was_airborne`-Flag-Tracking** (3-Schicht-Verteidigung — siehe §6.1)
7. **Pause/Slew-Freeze**: `if snap.paused || snap.slew_mode { return None }` — KEIN Phase-Wechsel waehrend Pause/Slew
8. Pro aktueller `stats.phase`: passende Transition pruefen → `next_phase`
9. Wenn `next_phase != stats.phase`: Side-Effects ausloesen, `phase = next_phase`, `record_event(PhaseChanged)`

---

## 3. Authority Model (Korrigiert v1.2)

Wer darf was setzen? Klare Trennung wichtig damit nicht mehrere Quellen das gleiche Feld konkurrierend schreiben.

| Komponente | Darf Phase setzen? | Darf Timestamps setzen? | Darf Sub-Score-Felder setzen? |
|---|---|---|---|
| **Streamer-Tick** (`step_flight`) | **Ja** (alle Phase-Wechsel inkl. Final→Landing) | `block_off_at`, `takeoff_at`, **`landing_at`** (kopiert aus `sampler_touchdown_at`), `block_on_at` | `bounce_count`, `landing_score` (klassifiziert) |
| **Touchdown-Sampler** (50 Hz) | **Nein** | **`sampler_touchdown_at`** (eigenes Feld!) | `landing_rate_fpm`, `landing_peak_vs_fpm`, `landing_confidence`, `landing_source` (alle via `finalize_landing_rate()`) |
| **Resume/Restore** | **Ja** (1:1 aus persistierter Phase) | Alle persistierten Timestamps inkl. `landing_at` UND `sampler_touchdown_at` | Alle persistierten Score-Felder |
| **Premium-X-Plane-Plugin** | Nein | `pending_td_premium_*` (intermediate, im pending-state) | Premium-VS/G im pending-state |
| **MQTT/Web/Monitor** | Nur **anzeigen**, nie setzen | Nur anzeigen | Nur anzeigen |

### 3.1 [VERDACHT] Sampler vs Streamer Race auf `landing_at` (Praezisiert v1.2)

`finalize_landing_rate()` setzt **NICHT** `landing_at` — es setzt `landing_rate_fpm`, `landing_peak_vs_fpm`, `landing_confidence`, `landing_source`. Der Sampler schreibt seinen TD-Zeitpunkt in `sampler_touchdown_at` (lib.rs:9529).

Der Streamer-Tick (`step_flight` Final→Landing-Pfad) liest spaeter `sampler_touchdown_at` und kopiert es nach `stats.landing_at` (lib.rs:11679, 11735, 12767, 12802). 

**Race-Verhalten (verifiziert v1.3):** Final→Landing-Pfad in `step_flight` schreibt `landing_at` in **zwei Schritten** (lib.rs:11679 + 11734):

```rust
// Schritt 1 (lib.rs:11679): UNCONDITIONAL erster Wert aus Snapshot-Buffer
let actual_td_at = stats.snapshot_buffer.iter()
    .find(|s| s.on_ground)
    .map(|s| s.at)
    .unwrap_or(now);
stats.landing_at = Some(actual_td_at);

// Schritt 2 (lib.rs:11734): wenn Sampler einen besseren Wert hat → ueberschreiben
if let Some(sampler_at) = stats.sampler_touchdown_at {
    stats.landing_at = Some(sampler_at);
}
```

Das ist **bewusst nicht idempotent** — der Streamer-Tick will den genauen Sampler-Wert wenn vorhanden, sonst Snapshot-Buffer-Fallback, sonst `now`. Race-Risiko nur wenn der Sampler erst NACH Schritt 2 seinen Wert setzt — dann bleibt `landing_at` auf Snapshot-Buffer-Wert (~5s schlechter).

**Komplette Aufzaehlung der `landing_at`-Schreibstellen (v1.4 praezisiert):**

| Pfad | Site | Verhalten |
|---|---|---|
| Final→Landing | lib.rs:11679 | **Direkt** — Snapshot-Buffer-Fallback (`actual_td_at` aus erstem on_ground-Sample im Buffer, sonst `now`) |
| Final→Landing | lib.rs:11734 | **Direkt** — wenn `sampler_touchdown_at.is_some()` → ueberschreibt mit dem genaueren Sampler-Wert |
| Arrived-Fallback | lib.rs:12767 | **Guarded** — innerhalb `if stats.landing_at.is_none() && stats.takeoff_at.is_some()` — wenn Sampler vorhanden, schreibt sampler_at als Rescue |
| Arrived-Fallback | lib.rs:12802 | **Guarded** — gleicher Block, else-Branch: schreibt `now` als allerletzter Fallback |

**Korrekt:** **2 direkte Writes im normalen Final→Landing-Pfad + 2 guarded Rescue-Writes im Arrived-Fallback** (beide im selben Guard-Block). Nicht "3 unconditional + 1 guarded" wie v1.3 sagte.

---

## 4. Critical Invariants (NEU v1.1)

Was MUSS immer gelten — wenn eine dieser Invarianten gebrochen wird, ist der Flight-State inkonsistent.

| # | Invariante | Wo gepflegt |
|---|---|---|
| **I1** (P2.4 Korrektur) | Nach **erstem** Takeoff darf `takeoff_at` nicht mehr ueberschrieben werden (z.B. durch T&G die Phase auf Climb revertiert). Code setzt `takeoff_at` in mehreren Sonderpfaden (Boarding-direct, TaxiOut-direct, TakeoffRoll-edge — lib.rs:11192/11337/11397) — der gemeinsame Schutz: einmal gesetzt, T&G-Pfad-Revert beruehrt es nicht | Streamer Takeoff-Sites + T&G-Pfad |
| **I2** (P1.1 Korrektur) | `landing_at` wird vom **Streamer** geschrieben (kopiert `sampler_touchdown_at` wenn vorhanden, sonst Streamer-Snapshot-Zeit). `finalize_landing_rate` schreibt es **NICHT**. | Streamer Final→Landing + 3 weitere Sites (lib.rs:11679, 11735, 12767, 12802) |
| **I3** | `block_off_at` < `takeoff_at` < `landing_at` < `block_on_at` (zeitliche Ordnung) | Aktuell **NICHT explizit gepruft** |
| **I4** | Phase-Wechsel passieren NIE waehrend Pause/Slew | `step_flight` Pause-Freeze (§2 Schritt 7) |
| **I5** | `was_airborne == true` darf nur nach `block_off_at.is_some() + agl > 50ft + < 30000ft + 2 Ticks Dwell` | `step_flight` was_airborne-Block |
| **I6** | `bounce_count` wird vom 50Hz-Sampler-Analyse berechnet, nicht vom Streamer-Counter | Forensik v2 |

### 4.1 [VERDACHT] I3 ist nicht explizit gepruft

Es gibt keinen Assert / Sanity-Check dass die Timestamp-Reihenfolge stimmt. Bei Resume mit defektem `active_flight.json` koennte z.B. `landing_at < takeoff_at` reinkommen und die PIREP-Anzeige verfaelschen. **Empfehlung:** Sanity-Check beim Restore + beim PIREP-Submit.

---

## 5. Soft vs Hard Phases (NEU v1.1)

Bewusste Klassifikation welche Phase-Wechsel "best effort" sind und welche absolut korrekt sein muessen.

### 5.1 Hard Phases (muessen exakt stimmen)

- **TakeoffRoll → Takeoff** (setzt `takeoff_at`, gilt fuer Block-Fuel/Distance-Calculation)
- **Final → Landing** (setzt `landing_at`, fuettert Forensik v2 und Score)
- **BlocksOn → Arrived** (loest Auto-Submit-Hook + Discord-Embed)
- **Universal Arrived-Fallback** (Schutzschicht — siehe §7)

### 5.2 Soft Phases (Anzeige-only, keine harte Score-Wirkung)

- **Cruise / Descent / Holding** — phpVMS mapped sie sowieso alle auf "ENR". Pilot sieht sie im Web als Anzeige, kein Score-Effekt.
- **Approach / Final** — Score-relevant nur insofern als Score-Window am 1000-ft-AGL-Punkt anfaengt (Stability-Gate, siehe v0.7.1 Spec). Aber kein Hard-Cutoff.
- **TaxiOut / TaxiIn** — beide phpVMS "TXI", kein Score-Effekt.

**Konsequenz fuer QS:** False-Positives bei Hard-Phases sind kritisch. False-Positives bei Soft-Phases sind UX-Verwirrung aber kein Daten-Schaden. **Test-Prios entsprechend setzen.**

---

## 6. Transition-Tabelle

Pro Phase: aktueller Trigger + Schwellen + bekannte Anti-Flicker. Spalte "Klasse" zeigt Soft/Hard aus §5.

| Von | Nach | Trigger | Schwellen | Anti-Flicker | Klasse |
|---|---|---|---|---|---|
| Preflight | Boarding | Auto bei flight_start (kein Sim-Check) | — | — | Hard |
| Boarding | Pushback | `on_surface && groundspeed > 0.5 kt && engines == 0` | 0.5 kt | — | Hard |
| Boarding | TaxiOut | `on_surface && groundspeed > 0.5 kt && engines > 0` | 0.5 kt | — | Hard |
| Pushback | TaxiOut | `tug_done (pushback_state==3) ODER powered_taxi (engines>0 && gs>3 kt)` nach DWELL | `PUSHBACK_DWELL_SECS=10` | 10 s Dwell | Hard |
| TaxiOut | TakeoffRoll | `on_ground && gs > 30 kt && engines > 0` | 30 kt | — | Hard |
| **TakeoffRoll** | **Takeoff** | `was_on_ground && !on_ground` (Edge!) + setzt `takeoff_at` | on_ground-Edge | — | **Hard** |
| Takeoff | Climb | `agl > 500 ft` | 500 ft AGL | — | Soft |
| Climb | Cruise | `\|VS\| < 200 fpm && agl > 5000 ft` | 200 fpm + 5000 ft | — | Soft |
| Climb | Descent | siehe §6.2 (3 Zweige) | 200 ft `lost_from_peak` Mindest-Schutz | — | Soft |
| **Cruise** | **Holding** | `check_holding_entry`: `\|bank\| > 15° && \|VS\| < 200 fpm` ueber `HOLDING_ENTRY_DWELL_SECS=90s` | bank 15°, VS 200 fpm, **90 s Dwell** | 90 s Dwell, Pending-Reset bei Bedingungs-Unterbrechung | Soft |
| Cruise | Descent | `VS < -500 fpm && lost_alt > 5000 ft` | 5000 ft Drop, 500 fpm | Lost-Alt-Schutz | Soft |
| Descent | Approach | `agl < 5000 ft && VS < 0` | 5000 ft AGL | — | Soft |
| **Approach** | **Holding** | gleiches `check_holding_entry` (low-altitude hold) | wie Cruise→Holding | 90 s Dwell | Soft |
| Approach | Final | `agl < 700 ft` | 700 ft AGL | — | Soft |
| **Holding** | **Approach/previous** | `bank \|VS\| Bedingungen brechen` ueber `HOLDING_EXIT_DWELL_SECS=30s`; Approach wenn echter Descent erkannt | 30 s Exit-Dwell | 30 s Dwell | Soft |
| Approach/Final | Climb (Go-Around) | `agl > lowest_seen + 150 ft && VS > 300 fpm` ueber 8s Dwell | `GO_AROUND_AGL_RECOVERY_FT=150`, `GO_AROUND_MIN_VS_FPM=300` | 8 s Dwell, "Lowest-AGL"-Tracker | Hard (T&G/GA) |
| **Final** | **Landing** | **Streamer-Tick** detektiert `!was_on_ground && snap.on_ground` (Snapshot-Edge), setzt Phase + `stats.landing_at = Some(actual_td_at)`. Sampler liefert nur den besseren `actual_td_at`-Timestamp (siehe §3.1) | on_ground-Edge | Streamer schreibt direkt; Sampler liefert genauen TD-Zeitpunkt | **Hard** |
| Landing | Climb (Touch-and-Go) | `agl > 100 ft && !on_ground && engines > 0` fuer 1 s | 100 ft AGL, 1 s Dwell | Reset Landing-Window | Hard |
| Landing | TaxiIn | `gs < 30 kt && on_ground` | 30 kt | — | Hard |
| TaxiIn | BlocksOn | `parking_brake && gs < 1 kt && on_ground` | 1 kt | — | Hard |
| **BlocksOn** | **Arrived** | `engines == 0 && parking_brake && on_ground && (now - block_on) >= 30s` | `ARRIVED_DWELL=30s` | 30 s Dwell | **Hard** |
| (jede) | Arrived (Universal-Fallback) | `was_airborne && on_ground && engines == 0` ueber 30s Dwell. **NICHT** `groundspeed < 1` — der Dwell misst "on ground + engines off", nicht echten Stillstand (siehe §8) | `ARRIVED_FALLBACK_DWELL=30s` | `was_airborne`-Gate | Hard |
| Arrived | PirepSubmitted | Manuell via `flight_file` Tauri-Command | — | — | Hard |

### 6.1 Cruise → Holding und Approach → Holding (KORREKTUR v1.1)

`check_holding_entry()` (lib.rs:2686) prueft:
- `bank_deg.abs() > HOLDING_BANK_THRESHOLD_DEG (15°)`
- `vertical_speed_fpm.abs() < HOLDING_VS_THRESHOLD_FPM (200 fpm)`
- Halten fuer `HOLDING_ENTRY_DWELL_SECS (90s)`
- Bricht eine Bedingung → `holding_pending_since = None` (Reset)

Exit aus Holding: gleiche Bedingungen invertiert + `HOLDING_EXIT_DWELL_SECS (30s)`. Ziel: brief level segments waehrend 360° Turn nicht als Exit werten.

### 6.2 Climb → Descent (KORREKTUR v1.1)

Drei Zweige in `step_flight` Climb-Branch (lib.rs:11460+):

```
let lost_from_peak = stats.climb_peak_msl.unwrap_or(0.0)
                       - snap.altitude_msl_ft as f32;

(a) standard_tod         = VS < -500 fpm  &&  lost_from_peak > 200 ft
(b) low_altitude_descent = VS < -100 fpm  &&  agl < 3000 ft  &&  lost_from_peak > 500 ft
(c) catchall             = lost_from_peak > sehr-viel  &&  agl < 2000 ft
```

`200 ft lost_from_peak` schuetzt gegen einzelne -600 fpm Ticks: ein Climb-Glitch ohne tatsaechlichen Hoehenverlust triggert nicht. **Mein v1.0-Verdacht "ein einzelner -600fpm Tick kippt" ist falsch.** Echter Verdacht: bei realem Hoehenverlust + Turbulenz koennte der Pfad zu frueh greifen.

### 6.3 [VERDACHT] Descent ist nicht reversibel

`Descent → Cruise` existiert nicht. Wenn ein Pilot Step-Climb (FL370 cruise → FL350 climb → FL370 cruise) macht, wird er beim Step-Up nicht mehr als Cruise klassifiziert. Bei Airliners egal (phpVMS macht ENR aus beidem), bei VFR/Training/Heli aber spuerbar. **Empfehlung:** UI sollte das als "Soft-Phase" behandeln.

---

## 7. Special Transitions: Touch-and-Go, Go-Around, Divert, Holding

### 7.1 Touch-and-Go

Nach `Landing` Phase:
- `agl > 100 ft && !on_ground && engines > 0` fuer 1 s Dwell
- → Phase revertiert auf `Climb`, Landing-Window wird zurueckgesetzt
- Touchdown-Event bleibt im `touchdown_events` Vec mit `kind: TouchAndGo`

### 7.2 Go-Around (KORREKTUR v1.1)

`check_go_around()` (lib.rs:2631) — Anti-Flicker-Pattern:
- `lowest_agl_seen` wird waehrend Approach/Final gemerkt
- Trigger: `agl > lowest_seen + GO_AROUND_AGL_RECOVERY_FT (150ft)` UND `VS > GO_AROUND_MIN_VS_FPM (300fpm)`
- Dwell: 8 s
- → Phase auf `Climb`, `go_around_count++`

(Nicht 200 ft / 500 fpm wie v1.0 sagte.)

### 7.3 Divert

Kein eigener Phase-Wechsel sondern eine "Hint":
- Wenn `!near_planned (>=2nm vom geplanten Arrival)` waehrend Landing/TaxiIn
- → `find_nearest_airports()` setzt `stats.divert_hint` mit actual+planned ICAO
- Phase laeuft normal weiter (kein dedizierter Divert-State)
- PIREP-Submit-Pfad behandelt Divert speziell (`update_pirep` mit `arr_airport_id` ueberschrieben)

### 7.4 Holding (KORREKTUR v1.1)

Holding ist real implementiert. Eintrittspfade:
- **Cruise → Holding**: `check_holding_entry` triggert (sustained banked + level)
- **Approach → Holding**: gleicher Detection-Pfad bei Approach-Hold

Exit:
- Bedingungen brechen ueber 30 s Dwell → zurueck zur vorherigen Phase
- ODER: echter Descent waehrend Hold erkannt → direkt auf Approach

**[VERDACHT] §7.4-Verdacht:** `check_holding_entry` triggert bei jedem sustained Turn mit |bank| > 15° + |VS| < 200 fpm. Das matched **echte Holds**, aber auch:
- **Procedure-Turns** (90°-Drehung mit konstanter Hoehe = oft 30-45 s, also UNTER 90s Dwell — okay)
- **Lange Vektoren** mit Standard-Rate-Turn (wenn ATC einen Pilot 5 Minuten lang in einem 10°/min Turn haelt)
- **Orbit-Training** (bewusstes Kreisen)
- **Pattern-Work** (kontinuierliche Turns im Pattern)

Mitigation in der Praxis: `HOLDING_VS_THRESHOLD_FPM (200 fpm)` fang Pattern-Work weil Pattern oft VS > 200 fpm hat (Steig/Sink im Downwind/Final). Aber Vektor + Orbit koennten faelschlich als Holding klassifiziert werden.

**Empfehlung:** Holding als Soft-Phase behandeln (siehe §5) — Anzeige ja, Score/Strafe nein. Aktueller Code macht das implizit (kein Score-Effekt), sollte aber explizit dokumentiert werden.

---

## 8. Universal Arrived-Fallback (KORREKTUR v1.2)

`step_flight` Universal-Branch — Schutzschicht fuer Faelle wo der normale `BlocksOn → Arrived`-Pfad nicht durchlaeuft (z.B. Pilot vergisst Parking-Brake).

```
conditions_basic = snap.on_ground && snap.engines_running == 0
Trigger: was_airborne && conditions_basic ueber ARRIVED_FALLBACK_DWELL_SECS=30s
       && stats.block_off_at.is_some()
       && pre_block_off == false
       && already_done == false
```

**WICHTIG (P1.3 v1.2 Korrektur):** Der Code prueft **NICHT** `groundspeed_kt < 1`. Der 30s-Dwell misst tatsaechlich "on_ground + engines == 0" — also "on ground UND Engines aus". Pilot der mit ausgeschalteten Engines auf der Bahn rollt (sehr ungewoehnlich aber moeglich) wuerde nach 30s als Arrived klassifiziert.

**Lessons Learned:** drei Live-Bugs vor diesem Fallback noetig:
- PMDG-B738 GSX-Repositioning loeste Arrived bei FL538-Glitch aus — Fix: `agl > 30000 ft` blockt `was_airborne`
- Pilot mit kurzer Pause vor Block-Off bekam Arrived — Fix: `block_off_at.is_some()` Pflicht
- Single-Tick-Glitch poisoned `was_airborne` — Fix: 2-Tick-Dwell

### 8.1 [VERDACHT] Fallback-Sicherheit (Praezisiert v1.2)

Fallbacks sind oft die Stellen wo "ploetzlich Flug beendet"-Bugs entstehen. Pruefen ob:

- **"Engines off while rolling after landing"**: Pilot rollt mit `gs > 0 && engines == 0` (z.B. einer-Engine-Shutdown nach Landing fuer Cargo-Stand-Approach). Nach 30s wird Arrived gefeuert obwohl noch nicht stationary. **Echter Code-Risiko-Punkt.** Empfehlung: `groundspeed_kt < 1` als zusaetzliche Bedingung in `conditions_basic` aufnehmen.
- Near-Arrival-Check: was wenn Pilot 20 km vom Ziel abrollt zum Cargo-Stand? Der Fallback hat zwei Pfade (near_planned und divert) — pruefen ob beide robust sind.
- Engines-Check robust: FENIX schaltet APU mal aus, was als `engines_running == 0` zaehlen koennte (separate APU-Signal-Pruefung im Code? — pruefen).

---

## 9. Anti-Flicker-Mechaniken

### 9.1 `was_airborne`-Flag (3-Schicht-Verteidigung)

`step_flight` was_airborne-Block — sticky Flag, einmal `true` bleibt sie. Setzen erfordert ALLE drei:
1. `agl > WAS_AIRBORNE_AGL_FT (50ft) && agl < WAS_AIRBORNE_AGL_MAX_FT (30000ft)`
2. `block_off_at.is_some()` (zeitlich plausibel)
3. Halten fuer `WAS_AIRBORNE_DWELL_TICKS (2)` Ticks

### 9.2 [VERDACHT] was_airborne Sticky-Reset

Sticky bedeutet: einmal `true`, bleibt `true` bis Flight-Ende. Wenn die 3-Schicht-Verteidigung doch durchbricht (z.B. neuer MSFS-Bug der konsistent FL40000 fuer mehrere Sekunden meldet), ist der Schutz weg fuer den Rest des Fluges.

**Wo nachschauen:** Gibt es einen "was_airborne reset" wenn die Bedingungen nicht mehr halten? `airborne_dwell_ticks = 0` wird gesetzt bei `airborne_now == false`, aber `was_airborne` selbst bleibt `true`. Bug oder Feature?

### 9.3 Engine-Anti-Flicker

`last_engines_running_above_zero_at` wird gestempelt jedes Mal wenn Engines > 0. Verschiedene Phase-Logiken nutzen diesen Timestamp um "Engines waren grade noch an" zu pruefen statt nur `engines == 0`.

### 9.4 Pushback-State-Tracking

`saw_pushback_state_active` wird sticky-true wenn `pushback_state != 3`. Verhindert dass kurze Glitches als "kein Pushback erkannt" durchgehen.

### 9.5 Bounce-Detection

Separate AGL-Edge-Logik:
- Arm: `agl > 35 ft` nach Touchdown gesehen
- Fire: `agl < 5 ft` gesehen → `bounce_count++`
- Window: 8 s nach Touchdown (`BOUNCE_WINDOW_SECS`)

### 9.6 Distance-Akkumulation im Holding/Pattern (NEU v1.1)

Distance wird Haversine pro Tick addiert. Im Holding zaehlt das die echt geflogene Strecke (z.B. 4nm Hold-Pattern × 12 Runden = 48 nm zusaetzliche Distance).

**Konzeptueller Punkt:** Die PIREP-Distanz ist "flown distance", nicht "route distance". Pilot der eine 100nm-Direct-Route fliegt aber 30 Min holdet, sieht im PIREP **148 nm** statt der 100 geplanten. Das ist technisch korrekt (echte Track-Distance), aber UX-mäßig erklärungsbedürftig.

**Empfehlung:** dokumentieren in der UI ("flown" vs "route") oder Holding-Distance separat ausweisen.

---

## 10. Side-Effects pro Transition

| Transition | Was wird geschrieben |
|---|---|
| Boarding → Pushback | `block_off_at = now`, MQTT `Block`-Event |
| TakeoffRoll → Takeoff | `takeoff_at = now`, `takeoff_pitch_deg/bank_deg/fuel_kg/weight_kg` |
| Cruise → Holding | (Anzeige-only, keine Side-Effects) |
| Approach → Holding | (Anzeige-only) |
| Holding → Cruise / Approach | (Anzeige-only — Exit zur vorherigen Phase, oder direkt Approach falls echter Descent waehrend Hold erkannt) |
| Final → Landing | Streamer setzt Phase + `landing_at` (zuerst aus Snapshot-Buffer, dann ueberschrieben mit `sampler_touchdown_at` falls vorhanden — siehe §3.1). `finalize_landing_rate()` setzt parallel `landing_rate_fpm/peak_vs/confidence/source` (NICHT `landing_at`). Touchdown-Window startet |
| Landing → TaxiIn | (kein direkter Score-Effekt). Landing-Score wird **im Landing-Window** finalisiert (kann vor/um TaxiIn herum passieren, nicht hart an die Phase-Transition gebunden — TOUCHDOWN_POST_WINDOW_MS gibt das Timing vor). Touchdown-Window-Dump triggert dann LandingFinalized + `landing_score_announced` |
| TaxiIn → BlocksOn | `block_on_at = now`, Activity-Log "Block on" |
| BlocksOn → Arrived | Auto-Submit-Hook (wenn aktiviert) |
| Arrived → PirepSubmitted | phpVMS `/file` POST + MQTT `Pirep`-Publish + Discord-Embed + landing_history.json |

Bei jedem Wechsel: `record_event(FlightLogEvent::PhaseChanged { from, to, at })` ins JSONL.

---

## 11. Resume / Pause / Restart

### 11.1 Persistence

`save_active_flight()` schreibt nach jedem Phase-Wechsel + alle 30 s:
- `<app_data_dir>/active_flight.json` mit `PersistedFlightStats` (alle Felder von FlightStats die Snapshot-relevant sind)
- Inkl. `phase`, `block_off_at`, `takeoff_at`, `bounce_count`, `landing_score`, `forensics_version`, `landing_confidence`, `landing_source` (v0.7.1+)

### 11.2 Restore

Beim AeroACARS-Start:
- Wenn `active_flight.json` existiert: `PersistedFlightStats.apply_to(&mut FlightStats)`
- **Phase wird 1:1 restored** — wenn der Pilot z.B. in Cruise war als die App geschlossen wurde, ist sie nach Restart in Cruise

### 11.3 Pause/Slew-Freeze (KORREKTUR v1.1)

`step_flight` hat einen expliziten Freeze:
```rust
if snap.paused || snap.slew_mode {
    return None;  // KEIN Phase-Wechsel
}
```

Distance/Fuel/Position werden VOR dem Freeze in den ersten Steps des Ticks aktualisiert — d.h. waehrend Slew laufen Distance und Position weiter. **Das ist Bug-Klasse §11.5.**

### 11.4 [VERDACHT] Erster Tick nach Resume / Pause-Exit

Wenn der Pilot AeroACARS schliesst waehrend er in Final ist, dann den Sim schliesst und 30 Min spaeter beides wieder oeffnet — die Phase ist `Final`, aber der Sim ist auf einem ganz anderen Flughafen. Der naechste `step_flight`-Tick wird die Phase aufgrund der neuen Snapshot-Werte normal weiter ausfuehren. Der Wechsel `Final → Landing` setzt einen Timestamp `landing_at` mit dem Sim-Snapshot-Timestamp — aber wenn der Sim "gerade wieder live ist" und der Pilot rein zufaellig auf einer Bahn rollt, wird das eventuell als Landing-Edge erkannt obwohl es nur Sim-Reload ist.

**Empfehlung:** "Sanity Tick" nach Resume — erster Snapshot nur validieren (= last_lat/lon setzen), keine Phase-Wechsel, kein Distance-Increment, kein Touchdown-Sampler.

### 11.5 [VERDACHT] Slew/Teleport vergiftet Distance

Wenn ein Pilot 300 nm slewt: Slew-Mode wird gemeldet → Phase-Freeze greift, ABER die Distance-Akkumulation passiert VOR dem Freeze. Resultat: 300 nm phantom-Distance im PIREP.

**Empfehlung:** Distance/Fuel/Position-Update auch hinter den Pause/Slew-Freeze stellen.

---

## 12. phpVMS-Status-Code-Mapping (KORREKTUR v1.1)

`phase_to_status()` (lib.rs:13759):

| Phase | Code | Phase | Code |
|---|---|---|---|
| Preflight | **BST** (gleicher wie Boarding) | Approach | APR |
| Boarding | BST | Final | FIN |
| Pushback | PBT | Landing | LDG |
| TaxiOut | TXI | TaxiIn | TXI |
| TakeoffRoll | TOF | BlocksOn | ONB |
| Takeoff | TKO | Arrived | ARR |
| Climb | ICL | PirepSubmitted | (None) |
| Cruise | ENR | | |
| Descent | ENR | | |
| **Holding** | **ENR** (kein dedizierter Code) | | |

phpVMS hat weniger Phasen als AeroACARS — Cruise/Descent/Holding alle ENR. **UI sollte nicht so tun als waere phpVMS die volle Wahrheit.**

---

## 13. Bekannte Bug-Klassen (v1.5 Status nach VPS-Daten-Analyse)

Status-Konvention:
- **🟢 VERDACHT** — nur Code-Audit, keine echten Daten
- **🔴 BELEGT** — durch echte VPS-Pilot-Daten bestaetigt (siehe §16)
- **🟡 VERMUTUNG** — durch VPS-Daten-Auffaelligkeit nahegelegt aber nicht eindeutig

### 13.1 🟢 Phase-Race-Conditions (§3.1)

**Verdacht (v1.4 praezisiert):** Sampler schreibt `sampler_touchdown_at`, Streamer kopiert daraus nach `landing_at`. Race-Klasse: Streamer-Tick koennte den Final→Landing-Edge sehen BEVOR der Sampler seinen `sampler_touchdown_at`-Wert validiert hat — Streamer schreibt dann `landing_at` aus dem Snapshot-Buffer (lib.rs:11679), Sampler kommt einen Tick spaeter mit besserem Wert. Pruefen ob die `sampler_at`-Override-Stelle (lib.rs:11734) verlaesslich greift oder ob `landing_at` auf dem Snapshot-Buffer-Wert haengen bleibt. **VPS-Daten zeigen kein Beispiel** — bisher nur Code-Verdacht.

### 13.2 🟢 Pause-Resume-Drift (§11.4)

**Verdacht:** Phase wird restored, aber kein Sim-Snapshot-Validation. Pilot der die App nach Final restartet ohne den Sim-Flug zu beenden koennte einen Phantom-Touchdown bekommen. **VPS-Daten haben 13 `flight_resumed` Events**, aber keinen mit Final-Restore + neuer Sim-Position — synthetischer Test noetig.

### 13.3 🟢 Slew/Teleport vergiftet Distance (§11.5)

**Verdacht:** Distance-Akkumulation passiert VOR Pause/Slew-Freeze (lib.rs:10926 vor lib.rs:11077). **VPS-Daten zeigen 0 Logs mit `slew_mode=true`** — Pilot hat Slew offenbar nicht benutzt oder Streamer skipt diese Snapshots. Bug-Klasse bleibt aber strukturell — synthetischer Test noetig.

### 13.4 🟢 Holding zu permissiv (§7.4)

**Verdacht:** `check_holding_entry` triggert auch bei langen ATC-Vektoren oder Orbit-Training. Anzeige-only, also kein Daten-Schaden, aber UX. VPS-Daten zeigen DLH742 mit 3 plausiblen Holdings (109s, 116s, 288s) — kein false positive sichtbar.

### 13.5 🟢 was_airborne Sticky (§9.2)

**Verdacht:** Einmal true, bleibt true. Wenn Schutz durchbricht, bleibt es vergiftet. Keine VPS-Daten-Auffaelligkeit.

### 13.6 🟢 Sonderpfade VFR/Heli/Glider/Seaplane (NEU v1.1)

Code hat Boarding-Direct-To-Takeoff-Pfade (laut Audit). Pruefen ob diese Flugarten alle wichtigen Timestamps + Distance + PIREP sauber bekommen. **VPS-Daten enthalten keinen Heli/Glider/Seaplane-Flug** — alle 29 Logs sind Airliner.

### 13.7 🟢 I3 Timestamp-Reihenfolge nicht gepruft (§4.1)

**Verdacht:** Bei Resume mit defektem `active_flight.json` koennte z.B. `landing_at < takeoff_at` reinkommen. VPS-Daten ohne Zeit-Anomalien gefunden.

### 13.8 🔴 Universal Arrived-Fallback Edge-Cases — BELEGT (§8.1)

**HISTORISCH BELEGT durch URO913** (siehe §16.1). Pilot rollte mit `engines_running=0` aber `groundspeed_kt = 141 → 42` ueber ~31.5 Sekunden — der Universal-Fallback feuerte trotzdem `Arrived` obwohl der Flieger noch rollte. Genau die Bug-Klasse die §8.1 als Verdacht beschrieb.

**Konkrete Fix-Empfehlung:**
```rust
// lib.rs:12680 erweitern
let conditions_basic = snap.on_ground
    && snap.engines_running == 0
    && snap.groundspeed_kt < 1.0;  // NEU: echtes "stationary"
```
Eventuell zusaetzlich `parking_brake` oder `phase in [Landing, TaxiIn, BlocksOn]` als zweiter Schutz.

**Regression-Test-Empfehlung:** URO913-JSONL als anonymisierte/minimierte Fixture in `tests/fixtures/phase_arrived_fallback_uro913.jsonl.gz`.

### 13.9 🔴 Holding-Pending leakt phasenuebergreifend — BELEGT (NEU v1.5)

**HISTORISCH BELEGT durch PTO105** (siehe §16.3). Pilot hatte `Approach → Holding → Approach` mit nur **5.2s** Holding-Aufenthalt — passt nicht zur 90s-Dwell-Regel von `HOLDING_ENTRY_DWELL_SECS`.

**Wahrscheinliche Ursache:** `holding_pending_since` (lib.rs:2693) wird beim Verlassen von Approach/Final/Climb-Pfaden NICHT zurueckgesetzt. Wenn der Pilot vorher in Cruise war und kurz auf Holding-Entry-Bedingungen kam (z.B. lange Vektor-Drehung) → `holding_pending_since` zaehlt schon Sekunden. Dann Wechsel auf Approach → Final → wieder Approach. Sobald die Holding-Bedingungen wieder kurz erfuellt sind, wird die akkumulierte Pending-Zeit gerechnet → Holding-Trigger nach 5.2s sichtbar.

**Konkrete Fix-Empfehlung:**
- `holding_pending_since = None` setzen bei jedem Phase-Wechsel der NICHT `→ Holding` ist
- Oder: Phase-Tracking-Marker im pending-state mitfuehren, beim Phase-Mismatch reset

**Regression-Test-Empfehlung:** PTO105-JSONL als Fixture in `tests/fixtures/phase_holding_leak_pto105.jsonl.gz`. Erwartung: Holding darf NICHT nach 5s triggern wenn `HOLDING_ENTRY_DWELL_SECS = 90`.

---

## 14. QS-Test-Matrix (13 Szenarien)

Statt riesiger Test-Liste — diese 13 Szenarien decken die wichtigsten Faelle ab. Wenn ein Szenario fehlschlaegt → Hotfix-Spec analog `aircraft-type-match`-Maintenance-Workflow.

| # | Szenario | Erwartung |
|---|---|---|
| **S1** | Airliner normal (A320 SimBrief-OFP, EDDF→EDDM, Hard-Phases sauber) | block_off_at < takeoff_at < landing_at < block_on_at, kein Phantom-Phase, Master-Score plausibel |
| **S2** | VFR Manual ohne ZFW (PA28, EDFE→EDDR-Pattern) | Loadsheet-Sub-Score skipped, kein 0-Penalty (v0.7.1 F1) |
| **S3** | Heli (H145) | Boarding → direkter Takeoff (kein Pushback), `was_airborne` bei niedriger AGL plausibel |
| **S4** | Glider (LS8 Aerotow) | Tow-Phase als TaxiOut/Takeoff klassifiziert? Distance plausibel? |
| **S5** | Seaplane (DHC-2 Beaver) | on_ground-Detection auf Wasser (Sim meldet nicht zwingend on_ground=true bei Wasser) |
| **S6** | Touch-and-Go (PA28 Pattern, 5x T&G) | Jeder T&G erkannt + Climb-Reset, kein Phantom-Final-Submit |
| **S7** | Go-Around (Airliner Approach 200 ft, dann Vollgas) | go_around_count++, Phase Final → Climb, kein PIREP-Submit |
| **S8** | Holding (5 Min ueber EDDF VOR) | Phase = Holding, Distance += echte Track (nicht 0), kein Score-Effekt |
| **S9** | Pause/Resume (Airliner in Cruise → Sim Pause 30 Min → Resume) | Phase bleibt Cruise, kein Phantom-Wechsel beim Resume-Tick |
| **S10** | Slew/Teleport (Airliner 300 nm slewen) | Phase bleibt, **`last_lat/lon` und `distance_nm` werden NICHT von Slew vergiftet** (siehe §13.3 — Slew-Schaden entsteht genau hier) |
| **S11** (NEU v1.2) | Engines off while rolling (Airliner Landing → engine-out shutdown bei gs=20 kt) | Universal-Arrived-Fallback feuert **NICHT** vor Stillstand (siehe §8.1 Code-Risiko) |
| **S12** (NEU v1.2) | Final restored, Sim auf anderem Flughafen (App-Restart waehrend Final, Sim laed neuen Flug) | **Vor User-Bestaetigung des Resume-Banners:** keine Transition (FSM-Tick darf nicht laufen). **Nach Bestaetigung:** ein "Sanity-Tick" muss verhindern dass der erste Snapshot Phantom-Landing triggert (z.B. neuer Sim-Standort auf Bahn → on_ground=true → Streamer wuerde `Final → Landing`-Edge sehen). Aktuell: **kein Sanity-Tick implementiert** (siehe §11.4 Verdacht), Bug-Risiko-Klasse |
| **S13** (NEU v1.2) | Holding zwei Faelle: (a) echter 5-Min-Hold ueber VOR, (b) langer ATC-Vector / Orbit-Training | (a) Phase = Holding nach 90s. (b) Bewusst akzeptieren als "false positive" ODER Code-Threshold anpassen — entscheiden in QS-Review |

### 14.1 Test-Empfehlung (v1.5 nach VPS-Analyse)

| Szenario | Test-Form | Daten-Quelle |
|---|---|---|
| **S1** Airliner Baseline | Real-Replay | VPS-JSONL (mehrere DLH/ITY/GTI-Logs) |
| **S6** T&G | Real-Replay | VPS-JSONL (PTO705) — heuristisch via `Climb` count > 1 |
| **S7** Go-Around | Real-Replay | VPS-JSONL (mehrere PTO-Fluege mit Approach→Climb-Sequenz) |
| **S8** Holding | Real-Replay | VPS-JSONL (DLH742 — 3 echte Holdings) |
| **S9** Pause | **Synthetisch** | Mock-Snapshots `paused=true` (kein VPS-Beleg) |
| **S10** Slew | **Synthetisch** | Mock-Snapshots `slew_mode=true` (kein VPS-Beleg) |
| **S11** Engines-Out-Fallback | Real-Replay | **VPS-JSONL (URO913 — historischer Bug)** — siehe §16.1 |
| **S12** Final-Restore Sanity-Tick | **Synthetisch** | Mock 2-Phase-Fixture (kein VPS-Beleg) |
| **S13(a)** Echter Hold | Real-Replay | VPS-JSONL (DLH742) |
| **S13(b)** Holding-Pending-Leak | **Real-Replay** | **VPS-JSONL (PTO105 — historischer Bug)** — siehe §16.3 |
| S2 VFR ohne ZFW | Manuell | VA-Owner mit Sim (kein VPS-Beleg) |
| S3-S5 Heli/Glider/Seaplane | Manuell | VA-Owner mit Sim (kein VPS-Beleg) |

### 14.2 Test-Strategie (NEU v1.5)

Empfohlene Reihenfolge:
1. **Zuerst** S11 (URO913-Replay) und S13(b) (PTO105-Replay) — beide historisch belegt, hoechster Aufwand-Nutzen-Wert
2. **Dann** S1, S7, S8 als Regression-Basis fuer die normalen Pfade
3. **Synthetisch** S9, S10, S12 — kontrollierte Mock-Snapshots, deterministische Asserts
4. **Manuell** S2-S5, S13(a) — VA-Owner-Validierung mit echtem Sim

**Daten-Hygiene:** JSONL-Replays muessen **anonymisiert/minimiert** werden bevor sie ins Repo committed werden:
- Pilot-IDs / Callsigns durch generische ersetzen (`PILOT01`, `TEST123`)
- Lat/Lon-Streams auf nur die relevanten Phase-Wechsel-Samples reduzieren (~50-100 statt mehrere tausend)
- `aircraft_registration`, `aircraft_title` neutralisieren wenn nicht testrelevant

---

## 15. VPS-Daten-Coverage (NEU v1.5)

### 15.1 Daten-Quellen

| Quelle | Inhalt | Geeignet fuer |
|---|---|---|
| **`recorder.db` SQLite** (28 MB) | `positions` (133k), `flight_sessions` (46), `pireps` (39), `touchdowns` (42), `flight_events` (673 — nur block+takeoff!) | Aggregierte Stats, Phase-Counts, Touchdown-Score-Audit |
| **JSONL-Logs** (`flight-logs/<va>/<pilot>/<pirep_id>.jsonl.gz`, 29 Files) | Komplette `SimSnapshot`-Streams mit allen Feldern + Phase-Changed-Events + Score-Events | **Bessere Quelle fuer Phase-QS** weil `engines_running`, `paused`, `slew_mode`, `simulation_rate`, `on_ground`, `gear_normal_force_n` etc. alle drin |

### 15.2 Wichtige Beobachtungen

- **DB.positions hat KEIN `engines_running` Feld** — fuer S11 Engine-Out-Detection nicht ausreichend, JSONL noetig
- **Keine `paused=true` / `slew_mode=true` / `simulation_rate != 1` in den JSONLs** — Pilot hat Pause/Slew offenbar nicht benutzt, ODER Streamer skipt diese Snapshots vor dem Upload (zu pruefen!)
- **`flight_events` Tabelle hat nur `block` und `takeoff`** — Phase-Wechsel werden NICHT in der DB persistiert, nur in den JSONL-Logs

### 15.3 Pilot-Datensatz-Inventur (8 GSG-Piloten, 29 Logs)

29 Logs mit kompletten Phase-Sequenzen — dominante Aircraft-Typen: B777F, A320, A330, A350, B738. **Keine Heli/Glider/Seaplane/VFR-Flugzeuge** — fuer S2-S5 muessen synthetische oder manuelle Tests ran.

### 15.4 Datenschutz

- JSONLs enthalten Pilot-IDs, Callsigns, Aircraft-Registrierungen, lat/lon-Strecken — **PII**.
- DB enthaelt zusaetzlich `provisioned_pilots.password` + `admin_users.password` als bcrypt-Hashes — bei Verbreitung Brute-Force-Risiko.
- **`.gitignore` blockiert** `pilot-pirep-data-*.zip` und `aeroacars-live-snapshot-*.db.gz` (seit v1.5 Patch).
- Replay-Test-Fixtures muessen vor Commit anonymisiert/minimiert werden (siehe §14.2).

---

## 16. Reale Regression-Kandidaten (NEU v1.5 — VPS-belegt)

### 16.1 🔴 URO913 — Universal Arrived-Fallback while rolling

**Was passierte:** Pilot landete, rollte mit gestoppten Engines aber hoher Groundspeed Richtung Cargo-Stand. Phase blieb auf `Pushback` (vermutlich falscher Phase-Wechsel davor — bleibt zu untersuchen). Nach ~31.5 Sekunden mit `on_ground=true && engines_running=0` aber `groundspeed_kt = 141 → 42` feuerte der Universal-Fallback `Arrived` — obwohl der Flieger noch rollte.

**Bug-Klasse §13.8:** Universal-Fallback misst nicht echtes Stillstand sondern nur "on_ground + engines off".

**Fix-Empfehlung:** `conditions_basic` um `groundspeed_kt < 1.0` erweitern (Code-Anker `step_flight`-Universal-Branch).

**Test-Empfehlung:** URO913-JSONL als Fixture `tests/fixtures/phase_arrived_fallback_uro913.jsonl.gz`. Erwartung: `Arrived` darf erst feuern wenn `groundspeed_kt < 1.0` ueber 30s anhaelt.

### 16.2 🟢 DLH742 — echtes Holding (positiv-Beleg)

**Was passierte:** Pilot fuehrte 3 echte Holdings durch: 109s, 116s, 288s. Alle drei wurden korrekt als `Cruise → Holding → Cruise` erkannt.

**Bedeutung:** Bestaetigt dass `check_holding_entry` Standard-Holds zuverlaessig erkennt. Kein Bug — nur Positiv-Beleg fuer S8/S13(a).

**Test-Empfehlung:** DLH742-JSONL als Fixture `tests/fixtures/phase_holding_positive_dlh742.jsonl.gz` fuer Regression-Schutz.

### 16.3 🔴 PTO105 — Holding-Pending-Leak

**Was passierte:** Pilot hatte `Approach → Holding → Approach` mit nur **5.2s** Holding-Aufenthalt. Das passt nicht zur 90s-Dwell-Regel von `HOLDING_ENTRY_DWELL_SECS`.

**Wahrscheinliche Ursache (siehe §13.9):** `holding_pending_since` (lib.rs:2693) wird beim Verlassen von Approach/Final/Climb-Pfaden NICHT zurueckgesetzt. Akkumulierte Pending-Zeit aus einer frueheren Phase kann spaeter sofort triggern.

**Fix-Empfehlung:**
- `holding_pending_since = None` setzen bei jedem Phase-Wechsel der NICHT `→ Holding` ist
- Oder: Phase-Tracking-Marker im pending-state mitfuehren

**Test-Empfehlung:** PTO105-JSONL als Fixture `tests/fixtures/phase_holding_leak_pto105.jsonl.gz`. Erwartung: Holding darf NICHT nach 5s triggern.

### 16.4 🟢 PTO705 — Go-Around / low-level-touch (positiv-Beleg)

**Was passierte:** Standard-Go-Around-Sequenz mit `Final → Climb` plus zusaetzliche low-level Touches. Wurde korrekt als Go-Around erkannt (`go_around_count > 0`).

**Bedeutung:** Positiv-Beleg fuer S7. Keine Auffaelligkeit.

**Test-Empfehlung:** PTO705-JSONL als Fixture `tests/fixtures/phase_go_around_pto705.jsonl.gz`.

### 16.5 Empfohlene Reihenfolge fuer Replay-Test-Implementation

1. **URO913-Fallback** (§16.1) — historisch belegter Code-Risiko-Punkt, hoechste Prio
2. **PTO105-Holding-Leak** (§16.3) — historisch belegt, mittlere Prio (UX-Bug, kein Daten-Schaden)
3. **DLH742-Holding-positiv** (§16.2) — Regression-Schutz fuer §13.4-Verdacht
4. **PTO705-Go-Around** (§16.4) — Regression-Schutz fuer S7
5. Spaeter: weitere DLH/ITY/GTI-Logs als Baseline-Regression-Fixtures

---

## 17. Glossar

- **Phase:** Wert von `FlightPhase` enum. Aktuell aktive Position im Flight-Lifecycle.
- **Transition:** Wechsel von einer Phase zur naechsten in `step_flight`.
- **Tick:** Ein Aufruf von `step_flight` ausgeloest vom Streamer-Worker (5-30 s je nach Phase).
- **Sampler-Tick:** Ein Aufruf vom Touchdown-Sampler (50 Hz waehrend Approach/Final/Landing).
- **Anti-Flicker:** Mechanik die verhindert dass kurze SimVar-Glitches Phase-Wechsel ausloesen (Dwell, Edge-Detection, Sticky-Flag).
- **Side-Effect:** Was waehrend einer Transition zusaetzlich passiert (Timestamp setzen, Event loggen, MQTT publishen).
- **Authority:** Wer darf welches Feld schreiben (siehe §3).
- **Hard Phase:** Phase-Wechsel der Score/Daten beeinflusst, muss exakt sein (siehe §5.1).
- **Soft Phase:** Phase-Wechsel der nur fuer Anzeige zaehlt, "best effort" reicht (siehe §5.2).
- **Pause/Slew-Freeze:** `step_flight` returnt None wenn `snap.paused || snap.slew_mode` — keine Phase-Wechsel.
- **Universal Arrived-Fallback:** Schutzschicht damit der Flight auch dann auf Arrived kommt wenn der normale BlocksOn-Pfad nicht durchlaeuft.
- **VERDACHT:** Markierung in dieser Spec fuer Code-Stellen die im Audit verdaechtig wirkten — keine bewiesene Bug aber sollte QS systematisch nachgehen.
- **flown distance vs route distance:** Distance ist Tick-Haversine = echte geflogene Strecke inkl. Holding/Vektoren. Nicht die SimBrief-OFP-Route-Distance.

---

**Ende der Spec v1.5 — VPS-Daten-Analyse eingebaut. 2 Verdachts-Punkte (§13.8 + §13.9) jetzt als 🔴 BELEGT markiert. §15/§16 als neue Sektionen. Naechster Schritt: Replay-Test-Implementation gemaess §16.5 — URO913-Fallback zuerst, dann PTO105-Holding-Leak.**
