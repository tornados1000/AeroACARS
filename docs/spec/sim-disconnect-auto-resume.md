# Sim-Disconnect Auto-Resume - Working Spec v0.4

**Status:** REVIEW / READY FOR MVP IMPLEMENTATION  
**Datum:** 2026-05-12  
**Quelle:** Verdichtet aus `Sim-Pause Handling - Master Spec v0.3` im Claude-Worktree.  
**Ziel:** Den echten AUA-323-Schmerz beheben, ohne das Team mit dem kompletten 35h-Masterpaket zu ueberfahren.

## Kurzfassung

Wir bauen **nicht** sofort das komplette Pause-Master-System.

Wir bauen zuerst einen kleinen, sicheren MVP:

1. Wenn der Sim mid-flight weg ist, darf der Flug weiter pausieren wie heute.
2. Wenn der Sim wieder Daten liefert, resumed AeroACARS automatisch.
3. Position-/Hoehen-/Sprit-Drift blockiert nie. Sie wird nur geloggt und ggf. angezeigt.
4. Reposition-Distanz darf nicht in `distance_nm` einlaufen.
5. Pausezeit wird vom Flight-/Block-Time-Wert abgezogen.
6. Der Server muss Sessions ueber `pirep_id` zusammenhalten, damit ein 20-Minuten-Loch nicht zwei Fluege erzeugt.

Alles andere aus v0.3 bleibt wertvoll, ist aber **nicht MVP**.

## Warum

Beim Incident **AUA 323 LOWW->ESGG** am **2026-05-11** ist der Sim im Descent eingefroren. Danach kamen ca. 23 Minuten keine Positionen. Nach manuellem Resume stand der Flieger schon am Boden in ESGG; der Server sah daraus zwei Sessions:

- Session A: langer Flug bis Descent, kein PIREP
- Session B: kurze Boden-/Arrival-Session mit PIREP

Das ist fuer QS, VA-Review und Pilot-Vertrauen schlecht. Der Pilot hat im Grunde denselben Flug weitergefuehrt, aber unser System hat ihn technisch zerschnitten.

## Leitentscheidung

**`pirep_id` ist die Flug-Identitaet.**

Nicht Position. Nicht Hoehe. Nicht Sprit. Nicht Zeit seit letzter Position.

Wenn dieselbe `pirep_id` aktiv ist, behandeln wir es als denselben Flug. Grosse Drift ist ein Hinweis fuer den Piloten und fuer den Audit-Log, aber kein Blocker.

## Aktueller Code-Stand

Der Client hat bereits:

- Sim-Disconnect-Erkennung nach `SIM_DISCONNECT_THRESHOLD_S = 30`
- `paused_since`
- `paused_last_known`
- manuellen Resume per `flight_resume_after_disconnect`
- Reset von `last_lat` / `last_lon` beim Resume, damit Reposition nicht in die Distanz laeuft
- Existing UI im Active-Flight-Panel fuer den Pause-Fall

Der aktuelle Schmerz ist also nicht „gar keine Pause-Logik“, sondern:

- Resume ist manuell und kann zu spaet passieren
- Pausezeit wird nicht sauber als Akkumulator behandelt
- Server-Sessions koennen durch lange Luecken getrennt werden
- Spec v0.3 vermischt MVP und Zukunftsausbau

## Zeitfenster und Crash-Faelle

Diese Zeiten sind fuer den MVP die Arbeitsannahme:

| Fall | Zeitfenster | Erwartetes Verhalten |
|---|---:|---|
| Sim liefert keine Snapshots mehr | 30 Sekunden | Client setzt `paused_since` und haelt den Flug lokal im Pause-State |
| AeroACARS laeuft weiter | 30 Sekunden Heartbeat-Takt | phpVMS-PIREP bleibt aktiv, obwohl der Sim gerade keine Positionsdaten liefert |
| Sim-Crash, AeroACARS bleibt offen | praktisch unbegrenzt | Heartbeat laeuft weiter; sobald der Sim wieder Daten liefert, soll Auto-Resume greifen |
| MSFS/X-Plane-Neustart, AeroACARS bleibt offen | praktisch unbegrenzt | gleicher Fall wie Sim-Crash; Drift wird geloggt, nicht geblockt |
| Blue Screen / Rechner-Reboot | bis ca. 6 Stunden nach letztem Heartbeat | Nach App-Start soll `active_flight.json` geladen und der offene phpVMS-PIREP per `pirep_id` wieder adoptiert werden |
| Rechner laenger als ca. 6 Stunden aus | nicht garantiert | phpVMS kann den Live-PIREP abgeraeumt haben; Client muss dann klar warnen statt still einen falschen neuen Flug zu erzeugen |

Wichtig: Die 6 Stunden sind kein Client-Timer, sondern das effektive phpVMS-/Server-Fenster nach dem letzten Lebenszeichen. Waehrend AeroACARS noch laeuft, verhindert der 30-Sekunden-Heartbeat genau diesen Ablauf.

### Was passiert bei Sim-Crash?

Wenn nur der Simulator abstuerzt oder neu gestartet wird, AeroACARS aber offen bleibt:

- nach 30 Sekunden ohne Snapshot wird der Flug pausiert
- AeroACARS sendet weiter Heartbeats an phpVMS
- der PIREP bleibt lebendig
- nach erneutem SimConnect-/X-Plane-Snapshot wird automatisch resumed
- Reposition-Distanz wird nicht in `distance_nm` gerechnet

Das ist der wichtigste und einfachste Rettungsfall.

### Was passiert bei Blue Screen / Rechner-Reboot?

Bei einem echten Rechner-Ausfall laeuft kein Heartbeat mehr. Dann zaehlt nur noch das Server-Fenster:

- **unter ca. 6 Stunden:** Resume/Adopt soll funktionieren, wenn `active_flight.json` noch da ist und phpVMS den PIREP noch kennt
- **ueber ca. 6 Stunden:** Resume ist nicht mehr garantiert; der Client soll einen klaren Recovery-/Expired-Hinweis zeigen

Das MVP-Ziel ist deshalb: Nach Neustart nicht blind neu starten, sondern zuerst den alten Flug anhand `pirep_id` wiederfinden.

## MVP Scope

### F1 - Auto-Resume aus bestehendem Pause-State

Wenn `paused_since.is_some()` und `current_snapshot(&app)` wieder `Some(snapshot)` liefert:

- Pause beenden
- Pause-Dauer berechnen
- Drift gegen `paused_last_known` berechnen
- `last_lat` / `last_lon` auf `None` setzen
- `paused_since` und `paused_last_known` leeren
- Activity-Log schreiben
- weiter normal streamen

**Keine Drift-Blocker.**

### F2 - Pause-Akkumulator

`FlightStats` bekommt:

```rust
pause_total_duration_secs: i64
pause_segments: Vec<PauseSegment>
```

Alle neuen Felder muessen `serde(default)` haben, damit alte `active_flight.json` weiter ladbar bleibt.

Bei Resume:

- Pause < 1 Sekunde ignorieren
- sonst `pause_total_duration_secs += duration`
- Segment speichern: Start, Ende, Reason, Drift Summary

### F3 - Flight-/Block-Time korrigieren

Beim Berechnen von Flight-/Block-Time:

```text
effective_duration = raw_duration - pause_total_duration
```

Wichtig: Das soll fuer PIREP-Zeiten gelten, nicht fuer echte UTC-Zeitstempel. Timestamps bleiben echt.

### F4 - Server-Session per `pirep_id` zusammenhalten

`aeroacars-live` muss bei eingehenden Events/Positionen mit `pirep_id` zuerst versuchen:

```text
find open or recent session by pirep_id
```

Erst wenn kein Match existiert, darf die alte Zeit-/Callsign-/DEP-/ARR-Heuristik greifen.

Das loest den AUA-323-Fall serverseitig: eine lange Positionsluecke erzeugt keine neue Session, solange die `pirep_id` gleich ist.

### F5 - Minimal-UI statt neuer UI-Welt

Fuer den MVP reicht die bestehende Pause-/Resume-UI plus Activity-Log:

- Quiet: nur Activity-Log
- Auffaellige Drift: Activity-Log als Warnung
- Sehr grosse Drift: bestehender Active-Flight-Bereich zeigt Warntext und Cancel-Hinweis

Keine neuen Toast-/Banner-Komponenten im MVP.

## Nicht MVP

Diese Punkte bleiben in der v0.3-Master-Spec wertvoll, werden aber bewusst verschoben:

- SimConnect `Paused` / `Unpaused` Events
- X-Plane-Plugin Paused-Heartbeat
- Replay-Erkennung
- Aircraft-Change-Banner
- Bid-Change-Detection waehrend Pause
- Drift-Linie auf Karte
- vollstaendige 29-Szenarien-Pilot-QS
- PIREP-Payload-Anzeige aller `pause_segments` in der Webapp

Grund: Das ist ein eigenes Paket. Wir brauchen jetzt erst die robuste Basis.

## Drift-Level fuer MVP

Nur fuer Log-Level und Text, nicht fuer Kontrolle:

| Drift | Level | Verhalten |
|---|---|---|
| < 1 NM | Info | `Flug automatisch fortgesetzt` |
| 1-50 NM | Info | `Flug automatisch fortgesetzt - repositioniert X NM` |
| 50-200 NM | Warn | `Auffaellige Reposition X NM` |
| > 200 NM | Warn | `Sehr grosse Reposition X NM - pruefe ob der richtige Flug geladen ist` |

Kein Level blockiert Resume.

## Datenmodell

### PauseSegment

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PauseSegment {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_secs: i64,
    reason: PauseReason,
    drift_nm: Option<f64>,
    altitude_delta_ft: Option<f64>,
    fuel_delta_kg: Option<f64>,
}
```

### PauseReason

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
enum PauseReason {
    SimDisconnect,
    ManualResume,
}
```

Fuer MVP reicht `SimDisconnect`. `ManualResume` kann optional entfallen, wenn es das Modell unnoetig verkompliziert.

## Akzeptanzkriterien MVP

| # | Kriterium |
|---|---|
| A1 | Sim liefert >30s keine Snapshots -> `paused_since` wird gesetzt |
| A2 | Sim liefert danach wieder Snapshot -> Client resumed automatisch ohne Button |
| A3 | Resume mit 0 NM Drift schreibt Info-Log |
| A4 | Resume mit 80 NM Drift resumed trotzdem und schreibt Warn-Log |
| A5 | Resume mit 500+ NM Drift resumed trotzdem und weist auf moeglich falschen Flug hin |
| A6 | Reposition-Distanz wird nicht zu `distance_nm` addiert |
| A7 | 10 Minuten Pause reduzieren PIREP-Flight-Time um 10 Minuten |
| A8 | App-Restart mit alter `active_flight.json` ohne neue Felder funktioniert |
| A9 | Server fuehrt zwei Positionsbloecke mit gleicher `pirep_id` in eine Session zusammen |
| A10 | Alter manueller Resume-Button bleibt als Fallback nutzbar |
| A11 | Rechner-Reboot unter ca. 6 Stunden adoptiert den offenen PIREP anhand `pirep_id`, wenn `active_flight.json` vorhanden ist |
| A12 | Rechner-Reboot ueber ca. 6 Stunden zeigt einen klaren Expired-/Recovery-Hinweis und startet nicht still einen falschen neuen Flug |

## Implementierungsreihenfolge

### Phase 1 - Client minimal

1. `FlightStats` um Pause-Akkumulator erweitern.
2. Helper `resume_from_pause_if_snapshot_available(...)`.
3. Bestehenden `is_paused`-Block im Streamer so aendern:
   - wenn Snapshot da: auto-resume
   - wenn kein Snapshot: weiter pausieren
4. Manueller `flight_resume_after_disconnect` nutzt denselben Helper.
5. Flight-/Block-Time-Abzug.
6. Unit-Tests fuer Drift und Pause-Akkumulator.

### Phase 2 - Server minimal

1. Sicherstellen, dass Client/MQTT/Recorder-Events `pirep_id` ausreichend mitgeben.
2. `findSessionByPirepId`.
3. `ensureSession` priorisiert `pirep_id`.
4. Test: 23-Minuten-Luecke bleibt eine Session.

### Phase 3 - UI polish

1. Text im bestehenden Pause-Hinweis anpassen: Auto-Resume laeuft, Button ist Fallback.
2. Bei grosser Drift deutlicher Cancel-Hinweis.

## Offene Entscheidungen

Nur diese Entscheidungen muessen vor MVP geklaert werden:

1. **Pausezeit abziehen:** Alle Pausen abziehen oder nur airborne?  
   Empfehlung: alle abziehen. Das ist konsistent und einfach.

2. **Warnschwelle:** Reichen 50/200 NM fuer Warnung?  
   Empfehlung: ja fuer MVP. Spaeter kann man phase-/streckenabhaengig werden.

3. **Server-Join:** Darf eine ARRIVED/gefilete Session mit gleicher `pirep_id` wieder geoeffnet werden?  
   Empfehlung: nein. `pirep_filed`/ARRIVED bleibt terminal. `pirep_id`-Join nur fuer nicht-terminale Sessions.

## Explizit nicht entscheiden jetzt

Damit der Kopf frei bleibt:

- keine Aircraft-Familienlogik
- keine X-Plane-Plugin-Version
- keine Karte/Drift-Linie
- keine komplette Toast-Banner-Welt
- keine 29 manuelle Szenarien vor dem ersten MVP

## Naechster konkreter Schritt

Vor der Implementierung einmal im Code verifizieren:

- Wo `flight_time_min` / `block_time_min` final berechnet werden
- Ob alle Position-/MQTT-Events die `pirep_id` enthalten
- Wie `ensureSession` aktuell terminale Sessions behandelt

Wenn das klar ist, kann Phase 1 gestartet werden.

## Entwicklungsanweisung Phase 1+2

Diese Anweisung ist der umsetzbare Arbeitsauftrag fuer die Entwicklung. Ziel ist ein gemeinsamer MVP aus Client-Resume und Server-Session-Join.

### Ziel

AeroACARS soll einen laufenden Flug nach Sim-Disconnect, Sim-Crash, MSFS/X-Plane-Neustart oder Rechner-Reboot sauber weiterfuehren, solange derselbe phpVMS-PIREP (`pirep_id`) gemeint ist.

Der Flug darf durch eine Positionsluecke nicht mehr in zwei Recorder-/Webapp-Sessions zerfallen.

### Scope

Implementiert werden nur diese Punkte:

1. Client erkennt Sim-Disconnect nach 30 Sekunden wie bisher.
2. Client resumed automatisch, sobald wieder ein Sim-Snapshot verfuegbar ist.
3. Client speichert Pause-Segmente und addiert `pause_total_duration_secs`.
4. Client zieht Pausezeit von Flight-/Block-Time ab.
5. Client setzt beim Resume `last_lat` / `last_lon` zurueck, damit Reposition-Distanz nicht gezaehlt wird.
6. Client sendet `pirep_id` in den relevanten Position-/MQTT-/Recorder-Payloads mit.
7. Recorder matched eingehende Positionen zuerst per `pirep_id`.
8. Recorder fuehrt offene/recent Sessions mit gleicher `pirep_id` zusammen.
9. Terminale Sessions duerfen nicht wieder geoeffnet werden.

Nicht Teil dieses Auftrags:

- neue Toast-/Banner-Architektur
- Drift-Linie auf Karte
- SimConnect Pause/Unpause Events
- X-Plane-Pause-Heartbeat
- Replay-Erkennung
- Aircraft-Wechsel-Blocker
- grosse Webapp-Pause-Segment-Ansicht

### Client-Aufgaben

Betroffener Hauptbereich:

- `client/src-tauri/src/lib.rs`

Umsetzung:

1. `FlightStats` erweitern:

```rust
pause_total_duration_secs: i64
pause_segments: Vec<PauseSegment>
```

Alle neuen Felder brauchen `#[serde(default)]` oder kompatible Defaults, damit alte `active_flight.json` weiterhin laden.

2. `PauseSegment` einfuehren:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PauseSegment {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_secs: i64,
    reason: PauseReason,
    drift_nm: Option<f64>,
    altitude_delta_ft: Option<f64>,
    fuel_delta_kg: Option<f64>,
}
```

3. Gemeinsamen Helper bauen:

```text
resume_from_pause_if_snapshot_available(app, flight, snapshot) -> ResumeResult
```

Der Helper soll:

- nur arbeiten, wenn `paused_since.is_some()`
- Pause-Dauer berechnen
- Drift gegen `paused_last_known` berechnen
- Pause-Segment speichern
- `pause_total_duration_secs` erhoehen
- `last_lat` / `last_lon` auf `None` setzen
- `paused_since` / `paused_last_known` leeren
- `active_flight.json` speichern
- Activity-Log schreiben

4. Streamer-Loop aendern:

- Wenn Pause-State aktiv und kein Snapshot da ist: weiter pausieren.
- Wenn Pause-State aktiv und Snapshot da ist: Helper aufrufen und danach normal weiterstreamen.
- Kein manueller Klick darf fuer den Normalfall noetig sein.

5. Manuellen Resume-Button behalten:

- `flight_resume_after_disconnect` bleibt als Fallback.
- Er soll denselben Helper oder dieselbe Kernlogik nutzen, damit Auto-Resume und manueller Resume nicht auseinanderlaufen.

6. Zeitberechnung korrigieren:

```text
effective_duration_secs = raw_duration_secs - pause_total_duration_secs
```

Das gilt fuer Flight-/Block-Time-Werte, die an phpVMS/PIREP gehen. UTC-Timestamps bleiben echte Zeiten und werden nicht verschoben.

7. Position-/MQTT-Payload pruefen:

- Sicherstellen, dass `pirep_id` bei laufendem Flug in den Payloads enthalten ist, die der Recorder fuer Session-Zuordnung sieht.
- Wenn `pirep_id` fehlt, Feld ergaenzen.

### Server-/Recorder-Aufgaben

Betroffene Hauptbereiche:

- `aeroacars-live/recorder/src/mqttSubscriber.ts`
- `aeroacars-live/recorder/src/db.ts`

Umsetzung:

1. In `ensureSession(...)` zuerst `pirep_id` pruefen:

```text
if payload.pirep_id exists:
    session = find reusable non-terminal session by pirep_id for this pilot
    if session exists:
        use session
```

2. Neue DB-Funktion bauen:

```text
findReusableSessionByPirepId(va_prefix, pilot_id, pirep_id)
```

Sie darf nur Sessions liefern, die:

- zum selben Piloten gehoeren
- dieselbe `pirep_id` haben
- nicht terminal beendet sind
- offen oder innerhalb des erlaubten Resume-/6h-Fensters liegen

3. Terminalschutz:

Sessions mit finalem Zustand wie `ARRIVED`, `FILED`, `pirep_filed` oder vergleichbaren `END_PHASES` duerfen nicht wieder als aktiv verwendet werden.

4. Fallback behalten:

Wenn kein `pirep_id`-Match existiert, bleibt die bisherige Heuristik aktiv:

- latest session
- Callsign/DEP/ARR/Aircraft-Kontext
- Zeitfenster

5. Optionaler Merge:

Wenn bereits zwei nicht-terminale Sessions mit gleicher `pirep_id` entstanden sind, darf der Recorder sie zusammenfuehren. Das ist hilfreich fuer Altfaelle, aber nicht zwingend fuer den ersten MVP, solange neue Luecken nicht mehr splitten.

### Verhalten bei Drift

Drift darf den Resume nicht blockieren.

| Drift | Verhalten |
|---|---|
| < 1 NM | Info-Log |
| 1-50 NM | Info-Log mit Distanz |
| 50-200 NM | Warn-Log |
| > 200 NM | Warn-Log mit Hinweis, richtigen Flug zu pruefen |

Auch bei >200 NM wird resumed, solange `pirep_id` passt.

### Verhalten bei Rechner-Reboot

Beim App-Start:

1. vorhandene `active_flight.json` laden
2. `pirep_id` aus aktivem Flug nutzen
3. pruefen/adoptieren, ob phpVMS/Recorder den PIREP noch kennt
4. wenn innerhalb ca. 6 Stunden seit letztem Heartbeat: Resume erlauben
5. wenn nicht mehr auffindbar/expired: klare Recovery-Meldung, keinen stillen neuen Flug erzeugen

### Tests / QS

Mindesttests fuer die Umsetzung:

1. Sim weg >30s -> `paused_since` wird gesetzt.
2. Sim kommt zurueck -> Auto-Resume ohne Button.
3. 10 Minuten Pause -> PIREP-Zeit ist 10 Minuten kuerzer als raw wall-clock.
4. Reposition 80 NM -> Resume + Warnung, Distanz wird nicht addiert.
5. Reposition 500 NM -> Resume + deutliche Warnung, kein Block.
6. App-Neustart mit alter `active_flight.json` -> keine Deserialisierungsfehler.
7. Recorder bekommt zwei Positionsbloecke mit gleicher `pirep_id` und 20-30 Minuten Luecke -> eine Session.
8. Recorder bekommt gleiche `pirep_id`, aber terminale Session -> keine Wiedereroeffnung.
9. Rechner-Reboot unter ca. 6h -> alter PIREP wird adoptiert.
10. Rechner-Reboot ueber ca. 6h oder PIREP nicht auffindbar -> klare Recovery-/Expired-Meldung.

### Definition of Done

- `npm run build` im Client gruen
- `cargo check` im Client gruen
- vorhandene Client-Tests gruen
- `npm run build` im Recorder gruen
- neue/angepasste Tests fuer Pause-Akkumulator und Session-Join vorhanden
- QS mit mindestens einem simulierten Disconnect und einer Server-Luecke dokumentiert

## Entwicklungsauftrag v0.7.15 - Sim-Recovery Release

### Release-Ziel

`v0.7.15` soll nicht nur ein kleiner Hotfix sein, sondern ein geschlossenes **Sim-Recovery-Release**.

Ziel ist: Ein laufender Flug soll nach Sim-Crash, Sim-Pause, Sim-Neustart, X-Plane-Pause, Aircraft-Wechsel nach Recovery und Rechner-Reboot so robust wie moeglich weitergefuehrt werden, ohne dass phpVMS oder der Recorder daraus falsche Zeiten, falsche Distanzen oder mehrere Sessions erzeugen.

### Bereits enthaltene Basis

Diese Punkte gelten als Basis und bleiben Bestandteil von `v0.7.15`:

1. Phase 1 Client Auto-Resume.
2. Pause-Akkumulator.
3. Pausezeit-Abzug von Flight-/Block-Time.
4. Reposition-Distanz wird nicht gezaehlt.
5. Heartbeat waehrend Sim-Disconnect-Pause mit `last_good_snap`.
6. `pirep_id` im Position-/MQTT-Payload.
7. Phase 2 Server-Join per `pirep_id`.
8. 6h-Cutoff fuer ended/recent Session-Reuse.
9. Terminalschutz gegen Wiedereroeffnen gefileter/angekommener Sessions.

### Zusaetzlicher Scope fuer v0.7.15

In `v0.7.15` sollen zusaetzlich umgesetzt werden:

| ID | Thema | Ziel |
|---|---|---|
| F5 | SimConnect Pause/Unpause Events | MSFS-Pause/Frozen-Snapshot sauber erkennen und Pausezeit akkumulieren |
| F6 | X-Plane Paused-Heartbeat | X-Plane-Pause aktiv melden, auch wenn weiterhin alte/stehende Daten kommen |
| F7 | Aircraft-Change-Banner | Nach Recovery warnen, wenn Sim-Flugzeug/Registration nicht mehr zum aktiven Flug passt |

`F8 Bid-Change-Detection` bleibt fuer `v0.7.15` nur optional als Light-Check. Wenn es den Release verzoegert, wird F8 verschoben.

### Nicht mehr in v0.7.15 aufnehmen

Damit der Release nicht ausufert:

- keine Drift-Linie auf Karte
- keine neue Toast-/Banner-Architektur
- keine Replay-Erkennung
- keine komplette Webapp-Pause-Segment-Ansicht
- keine 29-Szenarien-Voll-QS vor dem ersten Release
- keine grossen Refactors ausserhalb Recovery/Pause/Resume

### Implementierungsreihenfolge

#### Schritt 1 - F5 MSFS SimConnect Pause/Unpause

Ziel: Der Client soll echte MSFS-Pause erkennen, auch wenn SimConnect weiter eingefrorene Snapshots liefert.

Umsetzung:

1. Im MSFS/SimConnect-Adapter pruefen, ob Pause-State aus SimConnect verfuegbar ist.
2. Pause-State in den gemeinsamen `SimSnapshot` oder einen begleitenden Status einbauen.
3. Streamer-Loop so erweitern:
   - wenn Sim-Pause aktiv wird: `paused_since` setzen, `paused_last_known` speichern
   - waehrend Sim-Pause: Heartbeat weiter senden
   - wenn Sim-Pause endet: denselben Resume-Helper wie Auto-Resume nutzen
4. Keine harte Blockade bei Drift.
5. Pausezeit muss in `pause_total_duration_secs` einlaufen.

Akzeptanz:

- MSFS Esc-Pause 2 Minuten -> Pause-Segment ca. 120 Sekunden.
- phpVMS-Heartbeat laeuft waehrend Pause weiter.
- Nach Unpause resumed der Flug ohne Button.
- Flight-Time im PIREP ist um Pausezeit reduziert.

#### Schritt 2 - F7 Aircraft-Change-Banner

Ziel: Nach Resume/Recovery soll der Pilot sehen, wenn das aktuell geladene Flugzeug nicht zum aktiven Flug passt.

Umsetzung:

1. Beim Pause-Start vorhandene Aircraft-Daten speichern:
   - `aircraft_icao`
   - Registration, falls verfuegbar
   - ggf. Titel/Model-String, falls schon vorhanden
2. Beim Resume aktuellen Snapshot/Sim-Aircraft gegen den aktiven Flug vergleichen.
3. Bei Abweichung:
   - Warnung im bestehenden Active-Flight-Bereich anzeigen
   - Activity-Log schreiben
   - Resume nicht blockieren
4. Kein neues UI-System bauen. Bestehende Warn-/Banner-Flaeche verwenden.

Akzeptanz:

- Gleiches Flugzeug -> keine Warnung.
- Gleiche Familie/ICAO -> keine harte Warnung, maximal Info falls Registration anders.
- Anderes ICAO -> sichtbare Warnung.
- Warnung blockiert Resume nicht.

#### Schritt 3 - F6 X-Plane Paused-Heartbeat

Ziel: X-Plane soll einen aktiven Pause-Zustand an AeroACARS melden koennen, damit die Pausezeit auch dort sauber gezaehlt wird.

Umsetzung:

1. X-Plane-Plugin/Adapter pruefen, ob `sim/time/paused` oder aequivalenter Pause-State gelesen werden kann.
2. Pause-State in den Client transportieren.
3. Gemeinsame Pause-Logik wiederverwenden:
   - Pause aktiv -> `paused_since`
   - Heartbeat mit letzter Position
   - Pause Ende -> Resume-Helper
4. Falls Plugin-Protokoll erweitert werden muss:
   - backward-compatible Feld einfuehren
   - alte Plugin-Versionen laufen weiter ohne F6
   - UI zeigt keinen Fehler, sondern arbeitet wie bisher mit Disconnect-Erkennung

Akzeptanz:

- X-Plane Pause 2 Minuten -> Pause-Segment ca. 120 Sekunden.
- X-Plane alter Plugin-Stand ohne Pause-Feld -> kein Crash, Fallback wie bisher.
- Heartbeat laeuft waehrend Pause weiter.
- Flight-Time wird um Pausezeit reduziert.

### F8 Bid-Change-Detection Light Optional

Nur umsetzen, wenn nach F5-F7 noch sauber Zeit ist.

Light-Scope:

- Beim Resume pruefen, ob aktiver phpVMS-PIREP noch dieselbe `pirep_id` hat.
- Wenn ein anderer Bid/PIREP aktiv ist: Warnung und kein stiller Wechsel.
- Kein grosses Bid-Wechsel-UI bauen.

### QS-Matrix v0.7.15

| # | Bereich | Test |
|---|---|---|
| Q1 | Sim-Disconnect | Sim liefert >30s keine Snapshots -> Pause-State |
| Q2 | Sim-Disconnect | Ohne Snapshot sendet Client alle 30s Heartbeat mit `last_good_snap` |
| Q3 | Auto-Resume | Snapshot kommt zurueck -> Resume ohne Button |
| Q4 | Pausezeit | 10 Minuten Pause reduzieren PIREP-Zeit um 10 Minuten |
| Q5 | Distanz | 80 NM Reposition wird nicht zu `distance_nm` addiert |
| Q6 | Server | 25 Minuten Positionsluecke mit gleicher `pirep_id` bleibt eine Session |
| Q7 | Server | ARRIVED/PIREP_SUBMITTED Session wird nicht wieder geoeffnet |
| Q8 | Reboot | Rechner/App-Neustart unter 6h adoptiert offenen PIREP |
| Q9 | Reboot | Neustart nach >6h/PIREP weg zeigt Recovery-Hinweis |
| Q10 | MSFS Pause | MSFS Esc-Pause erzeugt Pause-Segment und Heartbeat laeuft weiter |
| Q11 | MSFS Pause | MSFS Unpause resumed automatisch |
| Q12 | X-Plane Pause | X-Plane Pause erzeugt Pause-Segment, wenn Plugin Pause-State liefert |
| Q13 | X-Plane Compat | Altes X-Plane-Plugin ohne Pause-State bleibt kompatibel |
| Q14 | Aircraft Change | Gleiches Aircraft nach Resume -> keine Warnung |
| Q15 | Aircraft Change | Anderes Aircraft nach Resume -> sichtbare Warnung, kein Block |

### Release-Gates v0.7.15

Vor Build/Release muessen gruen sein:

- Client: `npm run build`
- Client: `cargo check`
- Client: `cargo test`
- Client: vorhandene Frontend-Tests
- Recorder: `npm run build`
- Recorder: `npm test`
- Webapp: `npm run build`
- ein realer oder synthetischer MSFS Pause/Unpause-QS
- ein X-Plane-Kompatibilitaets-QS, mindestens mit altem Plugin-Fallback

### Release-Notes Entwurf

Titel:

```text
v0.7.15 - Sim Recovery Release
```

Deutsch:

```text
Diese Version verbessert die Wiederaufnahme laufender Fluege nach Simulator-Crash, Pause, Neustart oder kurzer Rechner-Unterbrechung.

- AeroACARS pausiert den Flug automatisch, wenn der Simulator keine Daten mehr liefert.
- Der phpVMS-PIREP wird waehrend einer Sim-Unterbrechung weiter per Heartbeat wachgehalten.
- Sobald der Simulator wieder Daten liefert, wird der Flug automatisch fortgesetzt.
- Pausezeit wird von Flight-/Block-Time abgezogen.
- Reposition-Distanz nach Recovery wird nicht als geflogene Distanz gezaehlt.
- Der Recorder haelt Sessions anhand der `pirep_id` zusammen, damit ein Flug nicht durch Datenluecken in mehrere Sessions zerfaellt.
- MSFS-Pause/Unpause und X-Plane-Pause werden, soweit vom Simulator/Plugin gemeldet, als echte Pause behandelt.
- AeroACARS warnt, wenn nach Recovery ein anderes Flugzeug geladen ist.
```

Englisch:

```text
This release improves recovery for active flights after simulator crashes, pauses, restarts, or short computer interruptions.

- AeroACARS automatically pauses the flight when the simulator stops sending data.
- The phpVMS PIREP is kept alive with heartbeat updates during simulator interruptions.
- The flight automatically resumes once simulator data returns.
- Paused time is subtracted from flight/block time.
- Reposition distance after recovery is not counted as flown distance.
- The recorder keeps sessions together by `pirep_id`, preventing one flight from being split by telemetry gaps.
- MSFS pause/unpause and X-Plane pause, where reported by the simulator/plugin, are treated as real pause time.
- AeroACARS warns if a different aircraft is loaded after recovery.
```

## Code-Check 2026-05-12

Diese Punkte wurden gegen den aktuellen Stand geprueft:

### Client

- `SIM_DISCONNECT_THRESHOLD_S = 30` existiert.
- `HEARTBEAT_INTERVAL = 30s` existiert; der Client haelt phpVMS waehrend laufender App regelmaessig wach.
- `paused_since` und `paused_last_known` existieren in `FlightStats`.
- `flight_resume_after_disconnect` existiert und macht bereits den wichtigen Reset von `last_lat` / `last_lon`.
- Der Streamer blockiert im Pause-State aktuell weiter und wartet auf manuellen Resume.
- `build_heartbeat_body(...)` berechnet `flight_time_secs` aktuell aus `takeoff_at`/`landing_at` bzw. `block_off_at`, aber ohne Pause-Abzug.
- Einige alte Code-Kommentare sprechen noch von `acars.live_time` / ca. 2h. Fuer den aktuellen Betrieb gilt als QS-Annahme ca. 6h nach letztem Heartbeat; die Kommentare sollten beim Implementieren bereinigt werden.

### Server

- `ensureSession(...)` in `aeroacars-live/recorder/src/mqttSubscriber.ts` priorisiert aktuell `latest` + `matchesFlightContext(...)` + `RESUME_WINDOW_MS`.
- `ensureSession(...)` liest aktuell keinen `pirep_id`-Wert aus dem Position-Payload als ersten Join-Key.
- `END_PHASES` existiert und schuetzt terminale Sessions.
- `findSessionByPirepForPilot(...)` existiert in `db.ts`, ist aber fuer Client-Log-Upload gedacht und nicht der benoetigte `ensureSession`-Join.
- `findActiveSessionForPilot(...)` nutzt serverseitig bereits ein 6h-Fenster fuer aktive Sessions.

### Konsequenz

MVP-Phase 1 kann rein clientseitig starten. Fuer den AUA-323-Server-Split braucht es zusaetzlich MVP-Phase 2:

1. `pirep_id` aus Position-/MQTT-Payload sicherstellen.
2. `db.findReusableSessionByPirepId(...)` bauen, mit Schutz gegen terminale Sessions.
3. `ensureSession(...)` ganz am Anfang auf diese Session matchen lassen.
