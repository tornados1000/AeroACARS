# Changelog

Alle nennenswerten Änderungen an AeroACARS. Format: lose an [Keep a Changelog](https://keepachangelog.com/) angelehnt; Versionsnummern folgen [Semantic Versioning](https://semver.org/) (Patch: Bugfix, Minor: Feature, Major: Breaking).

---

## [v0.3.1] — 2026-05-04

### Behoben
- **Divert-PIREP wurde fälschlich auto-akzeptiert.** phpVMS' `Acars\PirepController::file()` prüft beim Submit nur die Rang-Regel `auto_approve_acars` und ignoriert ein vorher gesetztes `source=MANUAL` auf dem PIREP-Record. Sobald der PIREP danach im Status `ACCEPTED` war, blockierte `checkReadOnly()` jeden weiteren State-Update — `state→PENDING` schlug mit „PIREP is read-only" fehl. Jetzt umgeht der Client den `/file`-Endpoint bei Divert komplett: ein einziger `update_pirep`-Call mass-assigned `state=PENDING`, `source=MANUAL`, `arr_airport_id`, alle Stats und Timestamps **solange der PIREP noch IN_PROGRESS ist**. Der PIREP landet sauber im PENDING-Bucket des VA-Admins ohne Auto-Approve-Trigger.
- **Activity-Log zeigte bei Divert die Plan-Destination statt der tatsächlichen.** „PIREP filed: RYR100 LOWS → EDDB" obwohl im PIREP `arr_airport_id=EDDP` stand. Jetzt: „PIREP filed: RYR100 LOWS → EDDP (DIVERT, planned EDDB)" — sowohl im Auto-Path als auch im Manual-Path.
- **CAVOK wurde im Wetter-Briefing nicht erkannt.** METAR-Texte mit `CAVOK` haben keine separaten Cloud-Layer-Codes, daher griff die alte Phänomen-Regex nicht. Jetzt wird CAVOK als eigenes Top-Level-Signal gerendert (☀ + Label).
- **Visibility-Anzeige sprang bei 9999 m nicht auf „≥ 10 km".** Schwelle war auf 10.0 km — `9999 m / 1000 = 9.999 km` rundete als „10.0 km" und blieb unter dem ≥-Operator. Schwelle jetzt 9.5 km, matched die Aviation-Konvention.

### Neu
- **Auto-Start-Skip-Banner im Briefing-Tab.** Wenn Auto-Start aktiv ist aber gerade nicht greifen kann (Triebwerke an, Flugzeug rollt, in der Luft), zeigt das Briefing einen gelben Banner mit der Begründung. Vorher musste der Pilot im Settings-Activity-Log nachschauen oder rätseln warum nichts passiert.
- **Auto-Start-State im Backend persistiert.** Bisher war `localStorage` die Source of Truth — nach Force-Kill / Hot-Reload im Tauri-Dev-Mode gelegentlich inkonsistent zum Watcher. Backend speichert den Toggle jetzt selbst (`app_config_dir/auto_start.json`); Frontend zieht beim Mount den Backend-Wert und syncht localStorage als reinen Cache.

### Intern
- `UpdateBody` (api-client) um `arr_airport_id`, `landing_rate`, `score`, `submitted_at`, `block_on_time` erweitert — nötig für den Divert-Mass-Assign-Pfad.
- `PirepFull.distance` entfernt — phpVMS gibt das Feld inkonsistent als Objekt oder Zahl zurück, wir brauchen's für den Status-Check eh nicht.

---

## [v0.3.0] — 2026-05-03

### Neu
- **Loadsheet-Feature.** Live-Anzeige von Block-Fuel / ZFW / TOW / Payload mit Δ-Vergleich gegen den SimBrief-OFP; Activity-Log-Eintrag bei Block-Off und Takeoff; „Über-Tankt"-Hint wenn Block-Fuel signifikant über Plan liegt.
- **SimBrief Soll/Ist-Vergleich.** PIREP-Detail zeigt geplante vs. tatsächliche Werte (Block-Fuel, TOW, Distance, Flight-Time) — auf einen Blick erkennbar wo der Flug vom Plan abwich.
- **SimBrief-Plan-Vorschau** im Briefing-Tab vor dem Flug-Start.
- **Aircraft-Anzeige** in Cockpit + Activity-Log: Type, Reg, Sim, Profil-Quelle (Standard / PMDG / etc.).
- **5/10-Schwellen** für Aircraft-Match: Toleranz beim Aircraft-Type-Vergleich gegen die Bid, damit z.B. A320 / A20N nicht fälschlich als Mismatch markiert werden.
- **OFP-Mismatch-Banner.** Erkennt wenn der zuletzt von SimBrief gelieferte OFP nicht zur aktuellen Buchung passt — Hinweis an den Piloten, einen frischen OFP zu generieren.
- **Auto-Start Activity-Log-Hint.** Wenn Auto-Start nicht greift (Triebwerke an, Flugzeug rollt, in der Luft), erscheint die Begründung im Settings-Activity-Log statt stillem Skip.

### Behoben
- **X-Plane Bug-Fixes.** Mehrere Korrekturen am UDP-DataRef-Adapter (Phase-FSM-Übergänge, Fuel-Einheiten, Touchdown-Detection bei niedrigen VS-Werten).
- **UX-Polish nach GSG-Live-Test.** Cockpit-Layout, Wetter-Briefing-Reihenfolge, OFP-Anzeige; Reaktion auf Pilot-Feedback aus dem ersten produktiven Einsatz beim German Sky Group VA.

### Intern
- v0.3.0 ist die erste Version mit Loadsheet-Pipeline (Stats-Capture bei Block-Off + Takeoff-Roll), die als Grundlage für künftige FOQA-/Performance-Auswertungen dient.

---

## Frühere Versionen

Notes für v0.2.x und v0.1.x liegen in den jeweiligen Release-Commit-Messages (`git log --oneline | grep release:`). Nachträglich migrieren wenn Bedarf besteht.
