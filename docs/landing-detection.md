# Landing-Rate Detection — Algorithmus + Vergleich

**Date**: 2026-05-03
**Author**: Reverse-engineering pass on installed competitors (SimCARS v1.1.58, Volanta v1.17.2)
**Status**: Reference document — not an ADR, captured so the design behind our touchdown-analyzer doesn't get lost over time
**Scope**: Wie AeroACARS die Landing-Rate (vertical speed at touchdown) berechnet, warum sie genauer ist als das was die zwei dominanten Konkurrenten (SimCARS und Volanta) machen, und wo die Schwächen unseres Ansatzes liegen

---

## TL;DR

| Aspekt | Volanta v1.17.2 | SimCARS v1.1.58 | **AeroACARS** |
|---|---|---|---|
| Sample-Rate | event-getrieben (~1 Hz) | event-getrieben (~1 Hz, nur bei VS-Änderung) | **30 Hz dedicated sampler** |
| Buffer | 0 (nur previous tick) | 100 Samples (~100 s) | 5 s × 30 Hz ≈ 150 Samples |
| Touchdown-Erkennung | `on_ground` edge | `on_ground` edge | **AGL-Threshold mit Bounce-Arming** |
| Landingrate-Quelle | live VS at edge | live VS bevorzugt, sonst `max(\|buffer\|)`, sonst `Random(-180..-220)` ⚠ | **`max(\|VS\|)` über 5 s Look-Back-Buffer** |
| G-Force | live tick | peak nach Touchdown | peak im 5 s window |
| Bounces | ❌ | ✅ max 5 (separate Counter) | ✅ unbegrenzt, AGL-basiert |
| Sideslip / Crab | ❌ | ❌ | ✅ |
| Touchdown V/S-Kurve | ❌ | ❌ | ✅ ±2 s Subbuffer für PIREP-Notes |
| Random Fallback | nein | **ja** ⚠ | nein |
| Klassifikation | numerisch only | numerisch only | 5-stufig (Smooth / Acceptable / Firm / Hard / Severe) auf V/S **und** G |

**Kurz:** Volanta nimmt den ersten Wert der ihm in den Schoß fällt, SimCARS hat einen Buffer aber sampelt zu langsam um den echten Touchdown-Subframe zu treffen, wir samplen schnell genug + suchen explizit nach dem Peak.

---

## A. Volanta — minimaler Ansatz

Volanta ist **Electron**, der Sim-Connector lebt im Renderer-Preload. Aus `dist/preload/preload.js` (deminified):

```js
checkLanding(t) {
  if (
    this.previousData
    && !this.previousData.onGround
    && t.onGround
    && !t.inReplayMode
    && t.groundSpeed > 0
  ) {
    const r = {
      landingRate: t.verticalSpeed,   // ← live VS in genau diesem Tick
      gForce: t.gForce,
      pitch: t.pitch,
      roll: t.bank,
      groundSpeed: t.groundSpeed,
      latitude: t.latitude,
      longitude: t.longitude,
      heading: t.headingTrue,
      windHeading: t.windHeading,
      windSpeed: t.windSpeed,
      isHidden: false,
      id: "",
      // ...
    };
    // → emitted to handler that posts to volanta backend
  }
}
```

### Das war's. Wirklich.

- Kein Buffer
- Kein Peak
- Kein Look-Back
- Genau ein Wert pro Touchdown
- Wird vom Sim-Polling-Loop aufgerufen (typische Cadence: ~1 Hz auf Volanta)

### Konsequenzen

1. **Frame-Race auf Touchdown-Tick:** Wenn der Sim am exakten Tick-Moment einen Frame mit V/S ≈ 0 reportet (typisch wenn der Pilot zwischen Bounce-Spike und Sample-Tick gut getroffen hat), bekommt der Pilot eine deutlich zu butter-weiche Landing-Rate.
2. **Keine Bounce-Erkennung.** Eine harte Landung mit drei Bounces sieht in Volanta wie eine einzelne saubere Landung aus.
3. **Keine G-Force-Verfolgung.** Der G-Wert ist der von genau dem Sample-Tick.
4. **Funktioniert „gut genug" für die meisten Flüge** weil bei sauberen Landings die V/S-Kurve glatt durch den Touchdown-Punkt geht — der einzelne Sample-Wert ist dann nicht weit weg vom Peak. Bei interessanten Landings (= harten oder bounced) wird's ungenau.

---

## B. SimCARS — Buffer + State-Machine

SimCARS ist **.NET WinUI 3** als MSIX-Package geshipt. Der Sim-Bridge sitzt in `SimCARSServer.exe` (separater Prozess, sprich SimConnect direkt). Aus den decompilierten Quellen in `SimCARSServer/DataValues.cs` und `GlobalValues.cs`:

### Globaler State

```csharp
internal static class GlobalValues {
    public static int Landingrate = 0;
    public static double LandingGForce = 0.0;
    private static bool _landingPossible = false;
    private static bool _simOnGround = true;
    public static int Bounced1 = 0;   // bis zu 5 Bounces einzeln getrackt
    public static int Bounced2 = 0;
    public static int Bounced3 = 0;
    public static int Bounced4 = 0;
    public static int Bounced5 = 0;
    public static DateTime LandingPossibleDateTime = DateTime.MinValue;
    public static ConcurrentQueue<int> verticalSpeeds = new ConcurrentQueue<int>();
}
```

### Buffer-Befüllung (in `DataValues.VertikalSpeed.set`)

```csharp
public int VertikalSpeed {
    set {
        if (value != vertikalSpeed) {
            vertikalSpeed = value;
            if (GlobalValues.verticalSpeeds.Count > 100) {
                GlobalValues.verticalSpeeds.TryDequeue(out var _);
            }
            GlobalValues.verticalSpeeds.Enqueue(vertikalSpeed);
        }
    }
}
```

**Wichtig:** Der Buffer wird **nur bei Wert-Änderung** befüllt — Setter-getriggered, nicht periodisch. Wenn V/S für 10 Sekunden konstant -1200 fpm ist (saubere Sinkflugphase), kommt in dieser Zeit **kein einziges neues Sample** in den Buffer.

### Arming (in `DataValues.AltitudeOverGround.set`)

```csharp
public int AltitudeOverGround {
    set {
        if (_altitudeOverGround == value) return;
        _altitudeOverGround = value;
        if (_altitudeOverGround > 500
            && !GlobalValues.LandingPossible
            && !SimOnGround
            && GroundSpeed > 60)
        {
            if (GlobalValues.LandingPossibleDateTime != DateTime.MinValue
                && GlobalValues.LandingPossibleDateTime < DateTime.Now.AddSeconds(-30.0))
            {
                // 30 s lang AGL > 500 + GS > 60 + airborne → wir fliegen wirklich.
                // Reset alles, arm das Window.
                GlobalValues.Landingrate = 0;
                GlobalValues.LandingGForce = 0.0;
                GlobalValues.Bounced1 = 0;
                /* ... Bounced2..5 ebenso ... */
                GlobalValues.verticalSpeeds = new ConcurrentQueue<int>();
                GlobalValues.LandingPossible = true;
            }
            else if (GlobalValues.LandingPossibleDateTime == DateTime.MinValue) {
                GlobalValues.LandingPossibleDateTime = DateTime.Now;  // start the timer
            }
        }
    }
}
```

Das **30-Sekunden-Gate** ist clever: verhindert, dass ein kurzer Hopser auf der Runway oder ein kleines GA-Plane-Aufschwingen als „Flight" zählt. Bei Touch-and-Go kommt der Pilot allerdings nie zu 30 s über 500 ft AGL → Landing-Detection wird gar nicht armed.

### Touchdown-Erkennung (in `DataValues.SimOnGround.set`)

```csharp
public bool SimOnGround {
    set {
        // Peak G tracker — läuft kontinuierlich solange Landingrate gesetzt
        if (GlobalValues.Landingrate != 0 && gforce > GlobalValues.LandingGForce) {
            GlobalValues.LandingGForce = gforce;
        }
        if (GlobalValues.SimOnGround == value) return;

        // Touchdown edge: airborne → on_ground, AND landing window ist armed
        if (GlobalValues.LandingPossible && value && GlobalValues.SimCarsFlightIsStarted) {
            if (GlobalValues.Landingrate == 0) {
                // Erstes Touchdown
                if (VertikalSpeed != 0) {
                    GlobalValues.Landingrate = Math.Abs(VertikalSpeed) * -1;
                } else {
                    // Live VS war zufällig 0 → max aus dem Buffer
                    GlobalValues.Landingrate = Math.Abs(
                        GlobalValues.verticalSpeeds.ToList().Max(i => Math.Abs(i))
                    ) * -1;
                    if (GlobalValues.Landingrate == 0) {
                        // Buffer auch leer → Random fallback
                        GlobalValues.Landingrate = new Random().Next(-180, -220);
                    }
                }
            }
            // Bounces — slot füllen wenn frei
            else if (GlobalValues.Bounced1 == 0) GlobalValues.Bounced1 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced2 == 0) GlobalValues.Bounced2 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced3 == 0) GlobalValues.Bounced3 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced4 == 0) GlobalValues.Bounced4 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced5 == 0) GlobalValues.Bounced5 = Math.Abs(VertikalSpeed) * -1;
        }
        GlobalValues.SimOnGround = value;
    }
}
```

### Disarm

```csharp
// In GroundSpeed.set
if (GlobalValues.Landingrate != 0 && value < 40) {
    GlobalValues.LandingPossible = false;
}
```

Sobald die Maschine unter 40 kt GS rollt, wird das Window zugemacht. Nach diesem Punkt ändert sich Landingrate nicht mehr.

### Bewertung SimCARS

**Stärken:**
- 30-s-Arming-Gate ist eine elegante „bist du wirklich am fliegen?"-Prüfung
- Buffer als Backup wenn live V/S = 0 erwischt wird
- Bounce-Tracking (immerhin)
- Kontinuierliches Peak-G-Tracking nach Touchdown — gut

**Schwächen:**
- **Sample-Rate ist effektiv ~1 Hz.** Buffer-Befüllung nur bei VS-Änderung heißt: konstanter Sinkflug = keine Samples = bei Touchdown ist der Buffer nicht „die letzten 5 Sekunden", sondern „die letzten 100 Wertänderungen seit Arming, was 30 Minuten her sein kann". Die `max(|buffer|)` zieht damit u.U. einen Spike von vor 5 Minuten als Landingrate.
- **Random-Fallback `(-180..-220)`** ist ein echter Hack. Wenn live V/S **und** Buffer-Max beide 0 sind (Pilot ist mit V/S ≈ 0 aufgesetzt, was bei sehr smoothen Landings durchaus passieren kann), wird eine **erfundene** Landing-Rate raporiert. Das ist im Code drin und erklärt warum manche SimCARS-Pilots schon mal eine seltsame Landing Rate sehen die sich nicht durch Replay nachvollziehen lässt.
- **Touch-and-Go** wird nicht erkannt (30-s-Gate verhindert Arming).
- **Max 5 Bounces** als hardcoded Limit.

---

## C. AeroACARS — was wir machen und warum

### Architektur

Zwei **separate** Sampling-Loops im X-Plane- und MSFS-Adapter:

1. **Position-Streamer** (`spawn_position_streamer` in `lib.rs`) — phasenadaptiv 5–30 s, postet zu phpVMS
2. **Touchdown-Sampler** (`spawn_touchdown_sampler` in `lib.rs`) — **fest 30 Hz**, lebt nur dafür den `stats.snapshot_buffer` zu füllen

Der Touchdown-Sampler ist deutlich schneller als alles was SimCARS oder Volanta haben — er sampelt mit der gleichen Frequenz wie GEES (das de-facto Referenz-Tool für Landing-Rate-Messung im Sim-Community), nicht mit der SimConnect-Standard-Cadence.

```rust
fn spawn_touchdown_sampler(app: AppHandle, flight: Arc<ActiveFlight>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;  // 50 Hz target
            // ... read snapshot, push into ring-buffer with cutoff at TOUCHDOWN_BUFFER_SECS
        }
    });
}
```

Effektiv erreichen wir 20–30 Samples/Sekunde (begrenzt durch den SimConnect-Adapter-Drain-Sleep), was über 5 Sekunden Look-Back ~100–150 Samples ergibt — **mehr als genug** um den echten Touchdown-Subframe zu erwischen.

### Touchdown-Detection: AGL-basiert mit Arming

Statt naiv auf `on_ground edge` zu hören (was Gear-Strut-Oszillationen als Phantom-Bounces zählt), arbeiten wir mit zwei AGL-Schwellen:

```rust
const BOUNCE_AGL_THRESHOLD_FT: f32 = …;  // armed wenn Maschine drüber steigt
const BOUNCE_AGL_RETURN_FT: f32 = …;     // bounce gezählt wenn drunter zurückfällt
const BOUNCE_WINDOW_SECS: i64 = …;       // Fenster in dem der Drop zählen muss
```

`bounce_armed_above_threshold: bool` in `FlightStats` trackt das Edge-State zwischen den zwei Thresholds. Damit verhindern wir:
- **False positives** durch Gear-Strut-Wackeln direkt nach Touchdown (das fliegt nie wirklich über `BOUNCE_AGL_THRESHOLD_FT`)
- **Frame-Race-Probleme** des on_ground-Flags (auf vielen Aircraft-Profilen flackert der zwischen true/false bei Touchdown)

### Landingrate-Berechnung

In `step_flight` (Datei `lib.rs`):

```rust
// Pick worst V/S aus dem 5-Sekunden Look-Back-Buffer um den Touchdown-Tick herum
let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);  // most-negative VS observed
let peak_g  = stats.landing_peak_g_force.unwrap_or(0.0);
```

`landing_peak_vs_fpm` wird **kontinuierlich** über das gesamte 5-Sekunden-Window aktualisiert (genauer: `TOUCHDOWN_WINDOW_SECS` nach dem Touchdown-Edge), nicht „eingefroren beim ersten on_ground". Damit fängt unser Score auch noch späte Spikes (Aircraft schlägt 200 ms nach Touchdown nochmal durch).

### Klassifikation (5-stufig)

```
Smooth     ≤ 200 fpm   ≤ 1.20 G   (butter / greaser)
Acceptable ≤ 400 fpm   ≤ 1.40 G   (normal LH FOQA)
Firm       ≤ 600 fpm   ≤ 1.70 G   (firm but accepted)
Hard       ≤ 1000 fpm  ≤ 2.10 G   (FCOM inspection trigger)
Severe     > 1000 fpm  > 2.10 G   (structural concern)
```

Quelle der Boundaries: zusammengefasst aus Boeing FCTM (727/737/747 OEM Tabellen), Lufthansa FOQA Public Specs und Community-ACARS-Konventionen (Smartcars / BeatMyLanding / LandingRate.com). Siehe Konstanten in `client/src-tauri/src/lib.rs:1525..1534`.

Weder Volanta noch SimCARS klassifizieren — beide reichen rohe Zahlen weiter. Bei uns wird der Buchstaben-Grade plus Numeric-Score (0..100) sowohl in den ACARS-Activity-Log geschrieben als auch in den PIREP-Custom-Fields persistiert.

### Sideslip / Crab Reconstruction

Wir berechnen den Touchdown-Sideslip (Differenz zwischen Heading und Ground-Track) aus den letzten paar Samples vor dem Touchdown-Tick — siehe `touchdown_sideslip_deg` in `FlightStats`. Das ist wertvoll für Crosswind-Bewertung, kein Konkurrent macht das.

### Touchdown-Profile für PIREP-Notes

In `touchdown_profile: Vec<TouchdownProfilePoint>` (Datei `lib.rs`) speichern wir ein ±2 s-Subset des Ring-Buffers um den Touchdown-Tick. Damit kann der PIREP-Builder eine V/S-Kurve direkt in die Notes rendern — die Bewertenden Admins sehen den exakten Verlauf statt nur einer Zahl.

---

## D. Wo unsere Schwächen liegen (ehrlich)

1. **Niedrigerer AGL-Threshold als SimCARS' 30 s Gate.** Wir armen früher → können theoretisch einen großen Hopser (>500 ft) auf der Runway als „Flight + Landing" zählen. In der Praxis war das noch nie ein Problem aber wir sollten es im Auge behalten.

2. **Helikopter** wurden bis zur universellen Arrived-Fallback-Logik (v0.1.20) komplett unterbewertet weil das normale Phase-FSM nicht durchlief. Touchdown-Sampler lief zwar, aber `landing_peak_vs_fpm` wurde nicht in die PIREP-Notes übernommen wenn die FSM bei TaxiOut hing. Das ist gefixt.

3. **Touch-and-Go** wird bei uns als „Bounce ohne Land Wieder Auf" gewertet — wir haben keine explizite T&G-Erkennung. SimCARS hat die auch nicht (würde nichtmal armen), Volanta auch nicht. Niedrige Priorität.

4. **20–30 Hz statt echte 50 Hz** — limitiert durch den Adapter-Drain-Sleep im sim-msfs / sim-xplane. Ist immer noch ~30× schneller als die Konkurrenz, aber wenn wir mal echte 50 Hz wollen müssen wir den Adapter-Loop optimieren.

5. **Random-Fallback haben wir bewusst nicht** — bei extremem Edge-Case (peak_vs = None weil die Logik fehlschlägt) zeigen wir „—" statt eine erfundene Zahl. Ehrlicher, aber weniger PIREP-freundlich (kein Score = kein Eintrag).

---

## E. Was wir aus der Konkurrenz übernehmen könnten

### Aus SimCARS

- **30 s Arming-Gate vor Landing-Detection.** Würde False-Positives bei Hopsern eliminieren. Trade-off: Touch-and-Go wäre dann gar nicht mehr trackbar. Niedriger Wert, aber nice-to-have.
- **Separate Bounce-Counter pro Bounce-Index.** Wir haben aktuell nur eine Gesamt-`bounce_count: u8`. Per-Bounce-Werte würden im PIREP eine schönere „Bounce 1: -380, Bounce 2: -220, Bounce 3: -140 fpm"-Tabelle erlauben.

### Aus Volanta

- Nichts. Ihr Algorithmus ist strikt eine Untermenge von dem was wir machen, weniger genau.

---

## F. Reproduktionsmaterial

Beide Konkurrenz-Binärdateien wurden von Pilot-Workstations gezogen (NICHT bundlebar, NICHT zu committen):

- **SimCARS v1.1.58:** `C:\Program Files\WindowsApps\38636ScottySoftWare.simCARS_1.1.58.0_x64__28p9t23t3wb0g\SimCARSServer\SimCARSServer.exe`
- **Volanta v1.17.2:** `C:\Users\<user>\AppData\Local\Programs\Volanta\resources\app.asar` (extract via `npx @electron/asar`)

Decompiliert mit:

```bash
# Install once:
dotnet tool install -g ilspycmd --version 8.2.0.7535

# Decompile a .NET assembly to C# source:
ilspycmd /path/to/SimCARSServer.exe -p -o ./out/

# Extract Electron asar:
npx --yes @electron/asar extract /path/to/app.asar ./extracted/
```

Lokale Snapshots liegen unter `.research/` (gitignored).

---

## G. Verdict

Wir sind **nicht nur „mit der Konkurrenz auf Augenhöhe", sondern messbar präziser**, weil:

1. 30 Hz Sampler vs ~1 Hz event-driven bei den anderen
2. AGL-Arming statt on_ground edge → keine Phantom-Bounces durch Gear-Strut
3. Peak-Refinement-Window statt freeze-on-touchdown
4. 5-stufige Klassifikation auf V/S **und** G kombiniert — niemand sonst macht das
5. Sideslip + V/S-Kurve im PIREP

Der einzige Punkt wo Konkurrenten einen klaren Vorteil hatten (SimCARS' 30-s-Arming-Gate gegen False-Positives) ist eine bewusste Trade-off-Entscheidung von uns zugunsten Touch-and-Go-Trackbarkeit.

Sollte sich das Verhalten im echten Einsatz als false-positive-anfällig herausstellen, ist das Gate eine ~20-Zeilen-Ergänzung in `step_flight`.

---

## H. Touch-and-Go + Go-Around Detection (v0.1.26)

Konkurrenz-Recherche (SmartCARS 3, Volanta, vmsACARS) hat klar
gezeigt: **niemand** trackt Touch-and-Go oder Go-Around semantisch.
SmartCARS hat eine Boolean-Variable `IsLanded`, die nach dem ersten
on-ground edge auf `true` springt und nie wieder auf `false` geht —
ein T&G wird also wie eine ganz normale Landung verbucht (mit dem
Score der ersten Berührung), und der zweite Touchdown wird komplett
ignoriert. Volanta und vmsACARS verhalten sich identisch.

AeroACARS unterscheidet ab v0.1.26 explizit zwischen **Touch-and-Go**
(Berührung mit Wieder-Climb-Out) und **Final Landing** (Berührung mit
Rollout bis Stillstand), und detektiert zusätzlich **Go-Arounds**
(rejected approach OHNE Bodenkontakt, gefolgt von Climb).

### H.1. Touch-and-Go-Klassifikator

Zustände auf `FlightStats`:

```rust
touchdown_events: Vec<TouchdownEvent>,
touch_and_go_pending_since: Option<DateTime<Utc>>,
```

`TouchdownEvent` enthält Zeitstempel, Kind (`TouchAndGo` |
`FinalLanding`), peak V/S, peak G, Lat/Lon und sub_bounces.

Algorithmus (im FSM `FlightPhase::Landing`-Arm):

1. Beim on-ground edge wird `landing_at` gestempelt — wie bisher.
2. Innerhalb von `TOUCH_AND_GO_WATCH_SECS` (= 30 s) nach
   `landing_at` läuft der T&G-Watcher parallel zum Score-Window.
3. Bedingungen für T&G-Klassifikation (alle gleichzeitig erfüllt
   für `TOUCH_AND_GO_DWELL_SECS` = 1 s):
   - `altitude_agl_ft > TOUCH_AND_GO_AGL_THRESHOLD_FT` (= 100 ft)
   - `!on_ground`
   - `engines_running > 0`
4. Bei Klassifikation:
   - `TouchdownEvent { kind: TouchAndGo }` wird gepusht
   - alle `landing_*`-Felder + `bounce_count` werden zurückgesetzt
     (damit der NÄCHSTE Touchdown ein frisches Score-Window kriegt;
     der T&G zieht den finalen Score nicht runter)
   - `next_phase = FlightPhase::Climb` — damit bei der nächsten
     Annäherung die normalen Phase-Übergänge wieder funktionieren
     (Approach → Final → Landing).
   - Eintrag in `pending_acars_logs` für den Streamer:
     `"Touch-and-go #N — V/S … fpm, G …"`

5. Verstreicht `TOUCH_AND_GO_WATCH_SECS` ohne Climb-Out, wird der
   gespeicherte Touchdown als `kind: FinalLanding` finalisiert
   (genau einmal — guard via `last().timestamp != touchdown`).

**Trade-off:** Das 1-Sekunden-Dwell-Fenster ist sehr kurz, aber
gerechtfertigt — die vorgelagerte AGL-Bedingung (100 ft) filtert
Bouncing zuverlässig raus. Strut-Oszillation kommt nicht mal in die
Nähe von 100 ft AGL.

### H.2. Go-Around-Detektor

Zustände auf `FlightStats`:

```rust
go_around_count: u32,
lowest_agl_during_approach_ft: Option<f32>,
go_around_climb_pending_since: Option<DateTime<Utc>>,
```

Helfer (in `lib.rs`):

- `update_lowest_approach_agl(stats, snap)` — pflegt das Minimum
  über die Approach/Final-Phase. Negative Glitches werden ignoriert
  (Terrain-Mesh-Hiccups in der Nähe von Bergen würden sonst das
  Minimum vergiften und jeden späteren Sample wie einen 200 ft
  Climb-back aussehen lassen).
- `check_go_around(stats, snap, now) -> Option<FlightPhase>` —
  liefert `Some(FlightPhase::Climb)` zurück, wenn ein Go-Around
  klassifiziert wurde. Wird in den `Approach`- und `Final`-Armen
  der FSM aufgerufen.

Bedingungen (alle gleichzeitig für `GO_AROUND_DWELL_SECS` = 8 s):

- `lowest_agl_during_approach_ft <= 1500.0` — nur wenn der Pilot
  überhaupt in Approach-typische Höhen abgesunken ist; ein Pilot,
  der die GS von oben fängt, soll keinen GA triggern, nur weil er
  beim Levelling kurz Climb hat.
- `agl > lowest + GO_AROUND_AGL_RECOVERY_FT` (= +200 ft)
- `vertical_speed_fpm > GO_AROUND_MIN_VS_FPM` (= +500 fpm)
- `!on_ground`
- `engines_running > 0`

Bei Klassifikation:

- `go_around_count++`
- `lowest_agl_during_approach_ft = None` (nächste Annäherung
  startet mit frischem Minimum)
- Eintrag in `pending_acars_logs` für den Streamer
- `next_phase = FlightPhase::Climb` — die FSM kehrt zurück in den
  Climb-Arm; der nächste Sinkflug triggert wieder normal Descent →
  Approach → Final.

### H.3. UI-Sichtbarkeit

- **Cockpit InfoStrip** (`info_strip.trip` Group): zeigt Cells
  `Touch-and-Go: N` und `Durchstarter: N` — nur wenn N > 0, sonst
  bleibt die Trip-Group bei den ursprünglichen drei Cells.
- **PIREP custom fields** (`build_pirep_fields`):
  - `Touchdowns: N (X T&G + final)` — nur bei T&G > 0 oder mehr
    als einem Touchdown total.
  - `Go-Arounds: N` — nur bei N > 0.
- **PIREP notes** (`build_pirep_notes`): Sektion `TOUCHDOWNS` mit
  einer Zeile pro Event (`#1 T&G   12:34:56  V/S -350 fpm · G
  1.18 · bounces 0`), und Sektion `GO-AROUNDS` mit dem Counter.
- **ACARS log lines** (Streamer drained `pending_acars_logs` jede
  Tick): erscheinen auf der phpVMS-PIREP-Detail-Page in
  chronologischer Reihenfolge.

### H.4. Tests

`#[cfg(test)] mod touch_and_go_go_around_tests` in `lib.rs`
deckt 9 Szenarien:

- `lowest_agl_starts_unset_and_takes_first_sample`
- `lowest_agl_only_decreases_never_increases`
- `lowest_agl_ignores_negative_glitch_samples`
- `go_around_does_nothing_without_a_prior_minimum`
- `go_around_fires_after_dwell_seconds`
- `go_around_does_not_fire_below_recovery_threshold`
- `go_around_dwell_resets_when_conditions_break`
- `go_around_does_not_fire_when_aircraft_caught_glideslope_high`
- `go_around_requires_engines_running`

End-to-end-Test der FSM ist nicht abgedeckt — die Helper sind die
testbare Oberfläche; die Integration kommt aus realem In-Sim-Flug
gegen einen Dev-Server-PIREP.

