# Q3 — Touchdown-Sampler testbar machen: Extraktions-Design (zur Review)

**Status:** ENTWURF, wartet auf Freigabe des Ansatzes. NICHT umgesetzt.
**Audit-Posten:** Q3 (Audit 2026-06-27). Sicherheitskritisch → Design-Review vor Umbau (Regel: exhaustive Analyse + `/code-review high`, kein patch-and-ship).

## 1. Problem
`spawn_touchdown_sampler` (`client/src-tauri/src/lib.rs:16774–17893`, ~1.100 Zeilen) ist eine async 50-Hz-Schleife, die:
1. den Sim pollt (`current_snapshot`), Reset-Detection macht, Samples in `stats.snapshot_buffer` assembliert,
2. Edge-/Touchdown-Detection fährt und die `touchdown_v2`-Kaskade aufruft,
3. Ergebnisse per MQTT publisht + Tauri-Events emittiert.

Die Replay-Tests (`tests/touchdown_v2_replay.rs`) extrahieren das **bereits fertige** `touchdown_window` aus den Fixtures und rufen `touchdown_v2` direkt. Sie testen also die Analyse, **nicht die Assembly** (Schritt 1+2) — genau den Teil mit der langen Bug-Historie (verpasster Touchdown, Go-Around, Arrived-Rescue, Bush-Capture-Reihenfolge). Dieser Pfad ist heute nur über die GUI erreichbar = ungetestet.

## 2. Ursache der Untestbarkeit
Die reine Decision-Logik ist mit I/O **verschachtelt**: `sleep().await`, `current_snapshot(&app)`, `Utc::now()`, MQTT-`publish`, `emit`. Man kann die Schleife nicht einfach aufrufen, weil sie einen `AppHandle` + laufenden Sim braucht.

## 3. Vorgeschlagener Seam: „Pure Core + Effects"
Die I/O **aus** der Decision-Logik herausziehen — Standard-Pattern für I/O-lastige Loops.

```
// Aller veränderliche Loop-State (heute lokale `let mut` + Teile von FlightStats):
struct SamplerState {
    prev_sample_for_reset_check: Option<(f64,f64,f32,DateTime<Utc>)>,
    reset_warning_logged: bool,
    edge_state: EdgeTrackingState,      // gear-force / on_ground edge
    snapshot_buffer: VecDeque<TelemetrySample>,
    touchdown: TouchdownCaptureState,   // FSM-/capture-/rollout-State
    // … alles, was die Schleife heute über Iterationen hält
}

// Beschreibt I/O, FÜHRT es nicht aus:
enum SamplerEffect {
    PublishMqtt(MqttTopic, serde_json::Value),
    EmitEvent(&'static str, serde_json::Value),
    LogResetWarn { reason: &'static str, /*…*/ },
    // …
}

// DER reine Kern — keine async, kein AppHandle, kein Sim, keine Uhr:
fn ingest_sample(
    state: &mut SamplerState,
    snap: &SimSnapshot,
    now: DateTime<Utc>,
) -> Vec<SamplerEffect>;
```

**Produktiv-Loop** (bleibt async, wird dünn):
```
loop {
    sleep(20ms).await;
    if stop { break; }
    let Some(snap) = current_snapshot(&app) else { continue };
    let now = Utc::now();
    for eff in ingest_sample(&mut state, &snap, now) {
        match eff { PublishMqtt(..) => …publish.await, EmitEvent(..) => app.emit(..), … }
    }
}
```

**Test** (treibt den **Produktiv-Pfad**, nicht einen Nachbau):
```
let mut state = SamplerState::new(sim, category);
for raw in fixture.raw_samples {        // die ROHEN Samples, nicht das fertige Window
    let _ = ingest_sample(&mut state, &raw.snap, raw.at);   // Effects im Test ignoriert/asserted
}
assert_eq!(state.touchdown.window(), fixture.recorded_touchdown_window);  // reproduziert?
```

## 4. Was wandert wohin
- **In den pure Core:** Reset-Detection (16834–16890), Buffer-Assembly (16892+), Edge-/Touchdown-Detection, `touchdown_v2`-Aufruf, Rollout-Tick, Arrived-Rescue, Latch-Logik.
- **Bleibt im Loop (I/O):** `sleep`, `current_snapshot`, `Utc::now`, MQTT-Publish, Event-Emit, Logging-Ausführung. (Der Core *beschreibt* sie als Effects.)

## 5. Validierungs-Strategie (Pflicht — Verhaltens-Erhalt ist #1)
1. **Fixtures brauchen die Rohsamples.** Heute liefert der Loader nur das fertige `touchdown_window`. Schritt 0: prüfen/ergänzen, dass die `.jsonl.gz` die rohe 50-Hz-Sample-Sequenz enthalten (sie sollten — es sind echte Flight-Logs). Falls nicht vollständig → Fixtures aus VPS-Logs nachziehen.
2. **Golden-Reproduktion:** Für JEDE bestehende Replay-Fixture muss `ingest_sample`-über-Rohsamples **dasselbe** `touchdown_window` (und denselben `touchdown_v2`-Verdict) erzeugen wie heute aufgezeichnet. Byte-/Feld-genau.
3. **Pure Move, kein Logikwechsel:** Die Extraktion ist ein reines Verschieben — keine Schwellen, keine Reihenfolge ändern. Diff-Review Zeile für Zeile.
4. **`/code-review high`** auf den Diff.
5. **Sim-Smoke** (Thomas): ein echter Flug X-Plane + ein MSFS, Touchdown wird korrekt erfasst — weil Schritt 1–4 headless nicht beweisen, dass die Live-I/O-Verdrahtung unverändert ist.

## 6. Risiko & Aufwand
- **Risiko:** hoch (sicherheitskritischster Pfad). Mitigiert durch „pure move + Golden-Reproduktion über alle Fixtures + code-review high + Sim-Smoke".
- **Aufwand:** L (1100 Zeilen entwirren; State-Struct sauber fassen; Effects-Enum; Fixture-Rohsample-Pfad; Tests). Kein Slice — aber in sich ein eigenes, reviewbares PR.
- **Nicht-Ziele:** Phase-Engine-v2-Cutover, 3-Resolver-Unification (separate Posten F2/F3).

## 7. Offene Fragen an Thomas
1. Enthalten die `tests/fixtures/*.jsonl.gz` die **rohen** 50-Hz-Samples (nicht nur das Window)? Wenn nein, dürfen wir 2–3 frische Fixtures von der VPS ziehen?
2. OK mit dem „Effects"-Pattern (Core gibt Effects zurück, Loop führt aus), oder lieber Callback-/Trait-Injection?
3. Soll das ein eigener PR sein (Branch + `/code-review high` + dein Merge), getrennt vom Härtungs-Release?
