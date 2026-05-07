# Changelog

Alle nennenswerten Änderungen an AeroACARS. Format: lose an [Keep a Changelog](https://keepachangelog.com/) angelehnt; Versionsnummern folgen [Semantic Versioning](https://semver.org/) (Patch: Bugfix, Minor: Feature, Major: Breaking).

---

## [v0.5.9] — 2026-05-07

🩹 **Climb→Descent FSM-Bug: ein einzelner VS-Spike beendete den Steigflug.**

Pilot Michael (MSFS, EGPH→HEGN B738): bei Climb auf FL050 hat ein einzelner -742 fpm-Spike (Level-Off-Maneuver) die FSM auf Descent geflippt. Aircraft stieg weiter durch FL340 und cruiste, aber FSM blieb 50+ Min in Descent hängen weil es keinen Descent→Climb Rücktransitionspfad gibt.

### 🐛 Behoben

Climb→Descent verlangt jetzt **zusätzlich** dass das Aircraft **echten Höhenverlust** vom Climb-Peak hat (>200 ft MSL).

```
Vorher: vs < -500 fpm                                    → Descent
Jetzt:  vs < -500 fpm AND lost_from_climb_peak > 200 ft → Descent
```

Single-Sample-Spikes (Turbulenz, Auto-Pilot-Trim, ATC-Level-Off) werden gefiltert. Erst wenn das Aircraft tatsächlich >200 ft Höhe verliert, gilt's als Descent. Echter Top-of-Descent verliert sofort tausende Fuß → triggert problemlos.

### 🛠 Intern
- Neues Feld `climb_peak_msl` in FlightStats (persistiert)
- Reset bei Takeoff→Climb (Re-Takeoff nach Divert)
- Wirkt für **MSFS und X-Plane** (FSM ist sim-agnostisch)
- Tests: 82 grün

---

## [v0.5.8] — 2026-05-07

🎯 **Multi-Window AGL-Δ + Plugin-Update — komplette Algorithmus-Konvergenz mit Volanta-Niveau.**

Pilot-Hinweis: „Volanta nutzt kein Plugin mehr und kriegt trotzdem korrekte Werte." Bestätigt unsere Strategie — der AGL-Δ-Algorithmus aus v0.5.7 ist self-sufficient ohne Plugin. v0.5.8 robustifiziert ihn weiter.

### 🆕 Multi-Window AGL-Derivative

Statt nur 2 s evaluiert der Client/Plugin jetzt **drei Fenster gleichzeitig** (1 s, 2 s, 3 s) und nimmt das negativste:
- **Hard Landing** (kein Flare): alle drei Fenster geben gleiche Werte
- **Airliner-Standard-Flare** (~3 s): 2 s-Fenster fängt den Pre-Flare-Sinkflug
- **GA Long-Flare** (~5 s): 3 s-Fenster deckt den relevanten Slice ab
- **Floater** (lange flache Approach): 1 s-Fenster misst nur die letzten Sekunden = sanfte Butter-Rate

### 🆕 Plugin (v0.5.8) — gleiche Methode

Plugin's Ring-Buffer hat jetzt auch AGL-Werte (war vorher nur VS+Pitch). Multi-Window-AGL-Δ läuft im Plugin self-sufficient. Kombiniert mit running airborne-VS-min als Backup.

**Aber wichtig:** Plugin ist optional. Volanta beweist dass die UDP-RREF-Daten von X-Plane (Port 49000) reichen — der Algorithmus macht den Unterschied, nicht der Plugin.

### 🛠 Intern
- Client: drei parallele AGL-Fenster, most-negative wins
- Plugin: VS-Buffer von 64 → 128 Samples (~3.8 s history bei 30 fps)
- Tests: 82 grün

---

## [v0.5.7] — 2026-05-07

🎯 **Methoden-Wechsel: VS wird jetzt aus AGL-Δ berechnet (LandingRate-1-Algorithmus, seit ~10 Jahren in der X-Plane-Welt erprobt).**

Pilot-Frage „warum kommen LandingRate.lua und Volanta immer auf richtige Werte und wir nicht?" — weil die einen fundamental anderen Ansatz nutzen den wir bisher nicht hatten.

### 🐛 Behoben

**Vorher** lasen wir die Sinkrate direkt aus `local_vy` / `vh_ind_fpm` (Flight-Model-Output). Beim Flare reduziert das Flight-Model die VSI absichtlich auf nahe 0 für gutes Stick-Feel — der Flieger sinkt physikalisch noch weiter, aber die VSI-Anzeige lügt schon. Egal wie clever wir Buffer-Min-Suche oder Running-Min nutzen, die Quelldaten sind kompromittiert.

**Jetzt** nutzen wir denselben Algorithmus wie LandingRate-1.lua (Dan Berry, 2014+) und Volanta:

```
gVS = (current_AGL - avg_AGL_letzte_2s) / (Zeitspanne / 2) * 60
```

Statt VSI lesen wir die **tatsächliche AGL-Differenz** über ein 2-Sekunden-Fenster. Das ist reine Geometrie — die Geometrie kann nicht durch Flight-Model-Tricks verfälscht werden. Bei einem Anflug von 81 ft AGL → 0 ft in 2 Sekunden gibt das exakt den echten Sinkflug, unabhängig von dem was VSI behauptet.

**Most-negative-wins** Hierarchie beim Final → Landing:
1. **AGL-Differential** (PRIMÄR — geometrische Wahrheit, wenn Sample-Density ausreicht)
2. Running Approach-Min (v0.5.5 Fallback)
3. Sampler-Edge-Capture (v0.4.4 Edge-Detection)
4. Buffer-Window-Scan (Legacy)
5. Live snap.vs (Last resort)

### 🛠 Intern
- Tests: 82 grün
- AGL-Daten waren schon im snapshot_buffer, kein neues Tracking nötig
- Wirkt mit ODER ohne Plugin (rein client-seitig)
- Plugin-Algorithmus folgt in v0.5.8 (gleicher Ansatz im C++)

---

## [v0.5.6] — 2026-05-06

🩹 **Plugin-Pendant zur v0.5.5-Touchdown-Logik.**

v0.5.5 hat den Bug im Tauri-Client gefixt; v0.5.6 fixt jetzt auch den Plugin-Code damit beide Schichten konsistent korrekt sind. Plugin sendet jetzt von sich aus den richtigen Wert.

### 🐛 Behoben

Plugin trackt jetzt auch eine **`g_airborne_vs_min`** — den negativsten pitch-korrigierten VS-Wert über den GESAMTEN airborne Segment (ground→air bis air→ground). Beim Touchdown-Edge wird der Wert mit dem Lookback-Window-Min und dem Live-VS verglichen — most-negative wins.

Zusammen mit der v0.5.5-Client-Logik gibt es jetzt **doppelte Korrektheit**:
- Plugin liefert von sich aus richtige `captured_vs_fpm` aus dem ganzen Anflug
- Client überschreibt nochmal mit dem eigenen Tracker falls Plugin doch falsch liegt

Reset-Logik im Plugin:
- Bei jedem ground→air Edge (Takeoff, Go-Around-Lift-off): Tracker = 0
- Nach erfolgreichem Touchdown-Capture: Tracker = 0 (Touch-and-Go bereit)
- Bei Plugin-Reload (`XPluginStop`): Tracker = 0

### ⚠️ Pilot-Aktion

1. v0.5.6 Auto-Update annehmen (Tauri-Client)
2. Settings → Debug → **„Plugin installieren"** klicken (lädt v0.5.6-Plugin)
3. **X-Plane neu starten** — neuer Plugin lädt erst beim X-Plane-Start

Dann ist das Plugin self-sufficient korrekt, auch ohne Client-Tracker-Override.

---

## [v0.5.5] — 2026-05-06

🩹 **Hotfix: Touchdown-VS bei aggressivem Flare wird endlich richtig erfasst.**

Pilot-Test (B738, MWCR Pattern, score 60/100 „firm" mit absurden Werten **VS +57 fpm bei G 1.52**): die Werte sind physikalisch widersprüchlich — 57 fpm = unmerklich, G 1.52 = harte Landung. Echte Sinkrate war ca. -500 fpm während des Anflugs (sichtbar im JSONL bei AGL 81 ft).

### 🐛 Behoben

Der 50-Hz-Sampler hatte ein zu schmales Lookback-Fenster (500 ms) und konnte bei aggressivem Flare nur **Post-Touchdown-Rebound-Samples** im Buffer finden — alle mit positivem VS. Resultat: das Min-Search fand keinen Sinkflug, gab den Rebound-Wert zurück.

**Doppelte Verteidigung in v0.5.5:**

1. **Running Peak-Descent-Tracker (`approach_vs_min_fpm`).** Ab Approach-Entry wird jeden 20-ms-Tick der **kleinste pitch-korrigierte VS-Wert** über die gesamte Approach + Final-Phase getrackt — unabhängig vom Sampler-Buffer. Selbst wenn X-Plane nur 1-2 Hz RREF liefert, fängt das den echten Peak-Sinkflug ein. Reset bei jedem neuen Approach (Go-Around-sicher).

2. **Sampler-Lookback erweitert von 500 ms auf 2 s.** Belt-and-suspenders gegen Buffer-Race-Bedingungen bei niedrigen RREF-Raten.

Beim Final → Landing wird nun der **negativste der drei Werte** genommen: Sampler-Edge-Capture vs. Buffer-Window-Scan vs. Running-Approach-Min. Most-negative wins.

### 🛠 Intern
- Tests: 82 grün
- Patch wirkt **mit oder ohne** installiertes X-Plane-Premium-Plugin — Plugin gibt frame-genaue Werte direkt vom flight-loop, der Tracker ist Backup für Plugin-lose Setups
- Persistierung des Trackers nicht nötig — er lebt nur innerhalb einer einzigen Approach-Phase

---

## [v0.5.4] — 2026-05-06

🩹 **Hotfix: Pattern-Flüge auf niedriger Höhe bleiben in Cruise hängen.**

Pilot-JSONL-Log: kurzer MWCR → MWCR Pattern-Test (B738), Cruise-Höhe 5000 ft AGL, 16 Min Flugdauer, normale Landung mit Aufsetzen — Ergebnis: keine Landing-Rate erfasst, Phase ging direkt von Cruise → Arrived.

### 🐛 Behoben

**Bug 1: Cruise→Descent forderte > 5000 ft Höhenverlust.** Der Cruise-Peak war bei 5002 ft MSL, beim Aufsetzen MSL 29 ft → Höhenverlust 4973 ft, **knapp unter** der 5000-ft-Schwelle. FSM blieb in Cruise, der Universal-Arrived-Fallback hat dann am Ende stumm direkt nach Arrived gesprungen — ohne durch Final→Landing zu gehen, also keine Touchdown-Erfassung.

Fix: Eskape-Klausel — Cruise→Descent triggert jetzt entweder bei (a) > 5000 ft Höhenverlust (wie bisher, für Airliner-TOD) **oder** (b) AGL < 3000 ft + Sinkflug (Pattern/GA-Bereich). Step-Downs bei FL360 lösen weiterhin keinen falschen Phasenwechsel aus.

**Bug 2: Universal-Arrived-Fallback verlor Touchdown-Daten.** Selbst wenn der 50-Hz-Sampler den Edge intern erfasst hatte, wurden VS/G nicht in den PIREP geschrieben weil der Code-Pfad „Final→Landing" der einzige war der das tat.

Fix: Rescue-Pfad — wenn Arrived-Fallback feuert UND der Sampler einen Touchdown gespeichert hat, werden `landing_rate_fpm`, `landing_peak_vs_fpm`, `landing_g_force`, `landing_peak_g_force` aus den Sampler-Werten gefüllt. Zweite Verteidigungslinie selbst wenn die FSM-Hauptkette ausfällt.

### 🛠 Intern
- Tests: 82 grün
- Beide Fixes wirken auch ohne installiertes X-Plane-Premium-Plugin (Sampler ist nativer Teil des Tauri-Clients)

---

## [v0.5.3] — 2026-05-06

🚨 **KRITISCHER Hotfix — Port-Konflikt mit X-Plane behoben.**

Pilot-Bericht mit Screenshot der X-Plane-Netzwerkeinstellungen zeigte: „Fehler bei der Initialisierung des UDP-Netzwerkausgangs (Port 49001). Lokales Netzwerk wird deaktiviert." Mein Plugin hatte 49001 für die Loopback-Kommunikation gewählt — **das ist aber X-Planes eigener Sende-Port**. Beide Apps stritten um denselben Socket → X-Plane konnte sein UDP-Netzwerk nicht initialisieren.

### 🐛 Behoben

- **Port von 49001 → 52000** in Plugin (`AEROACARS_UDP_PORT`) und Client (`PREMIUM_UDP_PORT`). 52000 ist:
  - **Weit außerhalb** X-Planes 49000-49003 Bereich (Send/Receive)
  - **Nicht** der X-Plane-Connect-Port (49520, NASA-Research-Tool)
  - In IANA Dynamic-Range, kein bekannter Service
  - Komplett konfliktfrei für 99,9% der Setups

### ⚠️ Pilot-Aktion erforderlich

1. AeroACARS-Update auf v0.5.3 installieren (auto-update)
2. Settings → Debug → Plugin **neu installieren** (lädt v0.5.3-Plugin von GitHub)
3. **X-Plane neu starten** — die Fehlermeldung über deaktiviertes lokales Netzwerk verschwindet, X-Planes UDP-Netzwerk arbeitet wieder normal

Plugin- und Client-Port müssen synchron sein — die v0.5.3-Auto-Install-Funktion zieht automatisch das passende Plugin-ZIP, daher reicht ein Klick auf „Plugin installieren" nach dem Client-Update.

### 🛠 Intern

- Neuer Defensive-Comment-Block in beiden Source-of-Truth-Konstanten warnt explizit vor X-Planes 49000-49003 Range
- Tests: 82 grün (unverändert)
- Plugin-Source ist nur an einer Konstante geändert, alle anderen Logiken stabil

---

## [v0.5.2] — 2026-05-06

🩹 **Hotfix: kein flackerndes Konsolen-Fenster mehr beim Settings-Tab-Klick.**

Pilot-Feedback nach v0.5.1: „beim Klick auf den Tab Einstellungen öffnet sich ein unsichtbares Fenster". Das war eine echte (leere) `cmd.exe`-Konsole, die kurz aufflackerte und den Fokus stahl — verursacht durch den `reg.exe query` aus der X-Plane-Pfad-Auto-Erkennung.

### 🐛 Behoben
- **`CREATE_NO_WINDOW`-Flag** für den `reg.exe`-Subprocess. Windows zeigt jetzt keine Konsole mehr an, kein Fokus-Stehlen, kein Flackern.

Patch nur Windows-relevant. Mac/Linux unverändert.

---

## [v0.5.1] — 2026-05-06

🩹 **Hotfix für v0.5.0-Regression — Settings-Tab hängt beim ersten Öffnen.**

Pilot-Feedback nach v0.5.0-Install: „Einstellungsseite ist hakelig beim Scrollen, Sprache konnte erst nicht verstellt werden." Klassischer Synchronization-Bug — der neue X-Plane-Premium-Panel rief auf seinem ersten Render einen synchronen Tauri-Command (`xplane_detect_install_path`) auf, der intern `reg.exe query` als Subprocess startete. Auf dem Main-Thread = blockiert den ganzen IPC-Kanal für ~200-800 ms, während dem **kein einziger anderer Command** durchkommt — daher Sprachwechsel-Hang + Scroll-Lag.

### 🐛 Behoben

- **`xplane_detect_install_path` ist jetzt async + `spawn_blocking`** — der `reg.exe`-Query läuft auf einem Worker-Thread, IPC bleibt frei, Settings-Panel reagiert sofort.
- **`xplane_uninstall_plugin` ebenfalls async** — beugt potenziellem Stall bei langsamen `remove_dir_all` (Windows Defender, Netzlaufwerke) vor.

### 🛠 Intern

- Selbe Pattern wie `detect_running_sim` (das schon seit v0.3.0 async ist).
- Tests: 82 grün (unverändert).

---

## [v0.5.0] — 2026-05-06

🚀 **„X-Plane Premium" — Frame-genaue Touchdown-Erfassung via nativem Plugin.**

Größtes Feature seit Release: ein optionaler nativer X-Plane-Plugin (XPLM SDK 4.3.0, C++17), der die Touchdown-Edge **innerhalb** des X-Plane-Flight-Loops erfasst — frame-genau, mit 500 ms Lookback-Buffer für die Peak-Sinkrate. Löst endgültig die seit v0.4.2 jagende „6 fpm Landing Rate trotz harter Landung"-Klasse von Bugs.

### 🆕 X-Plane Premium Plugin

**Was es tut:**
- Liest `fnrml_gear` (Gear-Normalkraft) jeden Frame und erkennt den exakten Frame des Aufsetzens (xgs-Methode, etablierte X-Plane-Konvention seit ~10 Jahren).
- Ermittelt die Peak-Sinkrate aus einem 500 ms-Lookback-Ring-Buffer **vor** dem Edge — so dass das gemessene VS dem tatsächlichen Anflug entspricht, nicht dem schon ausgependelten Wert nach Bodenkontakt.
- Pitch-Korrektur: `vs × cos(pitch)` (xgs-Konvention) — projiziert Welt-Y-Geschwindigkeit auf die Body-Achse.
- Sendet einen einmaligen JSON-„touchdown"-Paket über UDP an die AeroACARS-App auf `127.0.0.1:49001`.
- Re-armiert sich bei AGL > 50 ft, Touch-and-Go funktioniert also korrekt.

**Cross-Platform:**
- Windows x64 (`win.xpl`, MSVC, statisches CRT — keine DLL-Abhängigkeiten beim Piloten)
- macOS Universal (`mac.xpl`, x86_64 + arm64 in einer Datei)
- Linux x64 (`lin.xpl`, GCC)

**Sicherheit (NIE den Sim crashen):**
- Alle DataRef-Handles NULL-geprüft, alle Errors via `XPLMDebugString` geloggt, nie propagiert.
- Compile mit `-fno-exceptions -fno-rtti` (keine C++-Exceptions über die C-ABI-Plugin-Boundary).
- Non-blocking UDP `sendto()` — kein Stallen des Flight-Loops, auch nicht wenn der Client offline ist.
- Keinerlei Filesystem-Writes, keine Registry-Edits — Plugin ist read-only gegen X-Plane-State.
- Sauberes Reverse-Order-Cleanup in `XPluginStop`.

**Wire Format:** Versionierte Line-delimited-JSON über UDP-Loopback. Schema-`v:1`, zwei Pakettypen: `telemetry` (jeden Tick) + `touchdown` (one-shot pro Landung).

### 🆕 Auto-Install im AeroACARS-Client

Settings → Debug → „X-Plane Premium Plugin"-Karte:
- **Auto-Erkennung** des X-Plane-Hauptordners (Windows-Registry · macOS Standard-Pfade · Linux Standard-Pfade)
- **Manueller Pfad-Override** wenn die Auto-Erkennung nichts findet
- **„Plugin installieren"-Button** lädt die zur installierten Client-Version passende Plugin-Zip von GitHub und entpackt nach `<X-Plane>/Resources/plugins/AeroACARS/`
- **Status-Badge** „📡 live" sobald das Plugin Pakete sendet

### 🆕 Listener im Tauri-Client

- Neuer UDP-Listener (`crates/sim-xplane/src/premium.rs`) bindet `127.0.0.1:49001`, parst JSON-Pakete, surft Status + Touchdown-Events nach lib.rs.
- Touchdown-Sampler: wenn ein Premium-Paket eintrifft, **überschreibt** dessen `captured_vs_fpm` / `captured_g_normal` die RREF-basierte Edge-Detection — Frame-Genauigkeit, kein UDP-Eviction-Race mehr.
- RREF-Pfad bleibt voll funktional: Piloten ohne Plugin merken keinen Unterschied, ihre Flüge laufen wie vorher.

### 🛠 Intern

- Neuer Workspace-Member `xplane-plugin/` mit Cross-Platform-CMake-Build
- X-Plane SDK 4.3.0 vendored unter `xplane-plugin/third_party/XPSDK430/` (BSD-Lizenz, freie Commercial-Use)
- 6 neue Unit-Tests für den Premium-Packet-Parser
- 3 neue Tauri-Commands: `xplane_premium_status`, `xplane_detect_install_path`, `xplane_install_plugin`, `xplane_uninstall_plugin`
- GitHub-Actions-Pipeline erweitert: Plugin-Build-Matrix (Win/Mac/Linux) + Plugin-Package-Job, der die drei `.xpl` zu `AeroACARS-XPlane-Plugin-vX.Y.Z.zip` zusammenfasst und ans Release uploaded
- Bilingual i18n (DE+EN) für alle neuen Strings

### 🐛 Behoben (X-Plane only)

- **Landing-Rate-Bug aus v0.4.2/v0.4.3 final beseitigt:** Sampler-side Edge-Detection auf `fnrml_gear` (statt nur Streamer-side `on_ground`-Flag). Funktioniert sowohl mit als auch ohne Premium-Plugin — ohne Plugin macht der Sampler die Edge-Detection auf seinen 50-Hz-Snapshots, mit Plugin übernimmt das Plugin frame-genau.
- **Pitch-Korrektur bei VS-Capture:** Konsistent mit xgs (`vs × cos(theta_rad)`) im Sampler und im Plugin.

---

## [v0.4.3] — 2026-05-05

X-Plane-spezifischer Touchdown-VS-Fix nach Pilot-Live-Test heute Abend.

### 🐛 Behoben (X-Plane only)
- **Landing-Rate / peak_vs_fpm war bei X-Plane immer ~0** auch bei klar härteren Landungen. Pilot-Log heute (EWL6822 LEPA→EDDG, A320, sichtbare Sinkrate -350 fpm beim Aufsetzen): AeroACARS scorete „smooth, peak_vs_fpm: +5.7" — Touchdown-Window enthielt nur Post-Rollout-Daten.

  **Ursache:** Wir lasen `sim/flightmodel/position/vh_ind_fpm` — das ist die **VSI-Anzeige** wie im echten Cockpit, mit absichtlichem Damping (mehrere Sekunden Smoothing). Beim physischen Touchdown ist der gesmoothte Wert schon nahe 0, der echte Sinkflug ist als langsamer „Decay" über die letzten Sekunden verteilt — im 500ms-Touchdown-Window nicht mehr als Peak erkennbar.

  **Fix:** Switch auf `sim/flightmodel/position/local_vy` — die rohe vertikale Y-Achsen-Geschwindigkeit (m/s, real-time, kein Smoothing). Konvertierung im Setter: `value * 196.8504` (= 3.28084 ft/m × 60 sec/min). Das ist der gleiche DataRef den die etablierten X-Plane-Landing-Rate-Plugins (xgs, LRM, „A New Landing Rate Display") seit ~10 Jahren verwenden.

  Bei MSFS unverändert (ist ohnehin ein anderer Code-Pfad mit SimConnect-`PLANE TOUCHDOWN NORMAL VELOCITY`).

### 🛠 Intern
- DataRef-Switch in `client/src-tauri/crates/sim-xplane/src/dataref.rs`
- Verifiziert gegen X-Plane Developer-Doku + Production-Plugins (xgs, LRM)
- Tests: 76 grün

---

## [v0.4.2] — 2026-05-05

UX-Polish nach Pilot-Feedback heute Abend.

### 🆕 Neu
- **PIREP-Erfolgs-Banner** im Cockpit-Tab nach erfolgreichem File. Grüner Banner mit Callsign + Route bleibt 8 s sichtbar, manuell schließbar via X. Vorher verschwand das ActiveFlightPanel still — Pilot wusste nicht ob's wirklich gefilt wurde oder hängengeblieben ist.
- **Hint-Banner im Landung-Tab** wenn keine SimBrief-Plan-Werte vorhanden sind (alle SOLL-Spalten leer wären). Erklärt warum statt nur stille Striche.
- **SimBrief-OFP-Status im Activity-Log** beim Flight-Start. Vorher: Fetch-Fehler nur in Tracing-Logs (unsichtbar für Pilot). Jetzt drei Activity-Log-Einträge je nach Outcome:
  - ✅ „SimBrief OFP geladen" mit Plan-Block / Trip / TOW
  - ⚠️ „SimBrief-OFP konnte nicht geladen werden" mit OFP-ID + Erklärung
  - ⚠️ „SimBrief-OFP-Fetch fehlgeschlagen" mit Error-Detail
  - ℹ️ „Kein SimBrief-OFP für diesen Flug" wenn der Bid gar keinen hatte

So sehen Piloten beim nächsten Mal sofort wenn der OFP-Fetch silently scheitert (was zum heutigen „Landung-Tab hat keine SOLL-Werte"-Bug geführt hat).

### 🛠 Intern
- Neue UI-Component für PIREP-Success-Banner in `CockpitView.tsx` mit 8s Auto-Dismiss + Manual-Close
- i18n DE+EN für alle neuen Texte
- Tests: 76 grün

---

## [v0.4.1] — 2026-05-05

Real-Pilot-Workflow: was tun wenn der Sim mid-flight wegbricht?

### 🆕 Neu: Sim-Disconnect-Handling

Wenn der Streamer länger als 30 s keine brauchbaren Sim-Daten mehr bekommt (Sim-Crash, Quit, Window-Switch-Glitch), passiert jetzt:

1. **Flug wird automatisch in den Pause-Status gesetzt** — keine Position-Updates mehr an phpVMS, kein Phase-FSM, kein Activity-Log-Spam
2. **Die letzten bekannten Werte werden eingefroren** und sowohl im **Activity-Log** als auch in einem **gelben Banner im Cockpit-Tab** angezeigt:
   - Latitude / Longitude
   - Heading + Altitude
   - Fuel on Board
   - ZFW (Leergewicht)
3. **Heartbeat läuft weiter** — phpVMS' Live-Tracking-Cron killt den PIREP NICHT während der Pause (sonst wäre nach 2 h Schluss)
4. **„🔄 Flug wiederaufnehmen"-Button** im Banner — Pilot startet den Sim neu, lädt das Flugzeug an die richtige Position (oder bewusst andere — kein 5-NM-Restriction wie bei smartCARS), klickt den Button → Streamer macht weiter
5. **KEIN Auto-Resume:** auch wenn der Sim plötzlich wieder Daten liefert wartet das Backend auf den manuellen Klick (sonst würden Mid-Air-Position-Sprünge wild ins PIREP gehen)
6. **Reposition-Audit-Log:** beim Resume wird die Distanz zwischen alter und neuer Position berechnet. Bei großen Sprüngen (> 500 nm) als WARN-Level damit's für VA-Audits sichtbar ist
7. **Distance-Reset bei Resume:** Reposition-Sprung fließt **nicht** in die geloggte Flugdistanz ein. PIREP `distance_nm` zeigt nur tatsächlich geflogene Distanz, der Reposition-Δ wird separat als Activity-Log-Zeile festgehalten

Bewusst KEINE 5-NM/2000-ft-Restriktion wie bei smartCARS — der Pilot entscheidet wo er weitermacht, der Audit-Log macht's nachvollziehbar.

### 🛠 Intern
- Neuer Tauri-Command `flight_resume_after_disconnect` mit Δ-Distanz-Audit
- `FlightStats` erweitert um `paused_since` + `paused_last_known: PausedSnapshot`
- `ActiveFlightInfo` flow-through dieser Felder ans Frontend
- Neue Cockpit-Component `<DisconnectBanner>` (i18n DE+EN)
- Konstanten: `SIM_DISCONNECT_THRESHOLD_S=30`, `REPOSITION_WARN_DELTA_NM=500.0`
- Tests: 76 grün

---

## [v0.4.0] — 2026-05-05

Erstes Minor-Release der 0.4er-Reihe. Hauptthema: **Discord-Integration**.

### 🎉 Neu — Discord-Webhook

Vier Lifecycle-Events werden jetzt automatisch in den GSG-Discord-Channel gepostet, im Stil etablierter VA-Bots:
- ✈️ **Takeoff** (grün) — mit Block-Fuel + Plan-Δ + TOW
- 🛬 **Landung** (orange) — mit Landing-Rate + Score + Distance
- 📋 **PIREP filed** (violett) — kompletter Flugbericht
- ⚠️ **Divert** (amber) — mit Geplant/Tatsächlich-Vergleich

Layout angelehnt an den GSG-Bot-Stil:
- Author-Bar oben mit phpVMS-Pilot-ID + Name (z.B. „GSG0001 - Thomas K")
- Title als „Flight CHH3184/C.PF has landed"
- 3-Spalten-Felder: Dep.Airport / Arr.Airport / Equipment
- 2-Spalten-Felder: Flight Time / Distance
- **Großes Airline-Logo unten** — kommt direkt aus phpVMS (`bid.flight.airline.logo`), keine externe Hosting-Abhängigkeit. Wenn die VA das Logo-Feld in phpVMS pflegt, erscheint es automatisch.

Webhook-URL ist hardcoded für GSG (`#flights`-Channel). Posts laufen fire-and-forget (`tokio::spawn`) — Discord-Latenz blockt nie den Flugverlauf.

### 🛠 Intern
- Neues Modul `client/src-tauri/src/discord.rs` mit Embed-Builder + HTTP-Helper
- `ActiveFlight`/`PersistedFlight` erweitert um `airline_logo_url: Option<String>` (aus Bid-Relation; persistiert für Resume)
- `AppState.cached_pilot: Mutex<Option<(String, String)>>` — wird beim Login + Refresh aus dem phpVMS-Profile gefüllt, für die „GSG0001 - Pilot Name"-Zeile
- Discord Rich Presence Service (Crate `discord-rich-presence v1`) eingebaut aber noch nicht gewired — kommt in v0.4.1
- Tests: 76 grün

---

## [v0.3.5] — 2026-05-05

Drei X-Plane / phpVMS-Bugs nach Pilot-Test heute morgen.

### Behoben
- **MSL-Höhe weicht im Cruise um ~1.000 ft ab.** Wir lasen `sim/flightmodel/position/elevation` (= TRUE MSL, geographische Höhe über Sea Level), das aber bei nicht-ISA-Atmosphäre vom Indicated-Altitude abweicht. Pilot Michel D. sah heute auf YBBN→NWWW bei FL390 / OAT −46 °C → AeroACARS meldete 40.009 ft, PFD korrekt 39.000 ft (Differenz exakt die ISA-Deviation × 4 ft/°C). Im Sinkflug konvergierten die Werte wieder. Jetzt: `sim/cockpit2/gauges/indicators/altitude_ft_pilot` — Indicated, exakt was der Pilot sieht.
- **QNH-Anzeige zeigte unmögliche Werte (z.B. 198 hPa).** Der gelesene DataRef `sim/weather/barometer_current_inhg` ist der **Umgebungsdruck am Flugzeug**, nicht die Kollsman-Einstellung. Bei FL390 sind ~187 hPa Außendruck korrekt — aber das ist nicht was im Höhenmesser-Fenster steht. Jetzt: `sim/cockpit2/gauges/actuators/barometer_setting_in_hg_pilot` — die tatsächliche Altimeter-Setting (1013 hPa bei STD, real QNH bei lokal). Achtung: heißt `barometer_*` nicht `altimeter_*` (X-Plane-Naming-Inkonsistenz, gegen FlyWithLua + X-RAAS-Plugin verifiziert).
- **„Geflogene Route: 100%" während Boarding** auf der phpVMS-Live-Seite. v0.3.0 versuchte das durch Senden von `None` als `distance` während der Pre-Flight-Phase zu beheben — funktionierte nicht weil PHP's `empty()` sowohl `null` als auch `0` als „empty" erkennt und den 100%-Fallback triggert (1/1 = 100). Jetzt: minimaler Floor von 0.001 nm bis echte Distanz akkumuliert ist → `empty(0.001)` = false → Division läuft real → 0.001 / Plan-Distanz ≈ 0% bis Pushback.

---

## [v0.3.4] — 2026-05-04

Hot-Patch: v0.3.3 hatte einen TypeScript-Build-Fehler im CI (`'fnumMismatch' is declared but its value is never read`) — die Build-Jobs für Windows + macOS schlugen fehl, der `publish`-Step wurde geskipped, also kamen keine Installer am Release an. Inhaltlich = v0.3.3, nur sauber gebaut.

### Behoben
- **TS6133-Fehler in `BidsList.tsx`** — Cross-Product-Match-Logik entfernt nachdem v0.3.3 sie aus `ofpMismatch` rausgenommen hatte; die Variablen waren danach unused. Strict-Mode tot.

---

## [v0.3.3] — 2026-05-04 *(broken release — keine Build-Artefakte)*

Patch nach v0.3.2 — drei kleine UX-Fixes rund um die OFP-Mismatch-Detection.

### Behoben
- **Falscher OFP-Mismatch-Banner bei legitimen Plan-Varianten.** Der Match zwischen Bid-Flugnummer und SimBrief-OFP-Callsign war zu strikt. Beispiel: Bid „EWL 4368", OFP-Callsign „EWL4TK" (Pilot nutzt persönlichen ATC-Callsign in SimBrief). Der Banner feuerte fälschlich „SimBrief-OFP passt nicht zur Buchung", obwohl Aircraft + Origin + Destination alle übereinstimmten. Match-Logik jetzt bidirektional als Cross-Product aller Bid-Variants (Flight-Number + Callsign, mit/ohne Airline-ICAO-Prefix) gegen alle OFP-Variants. Plus: Flight-Number-Diff alleine triggert NICHT mehr den Banner — Aircraft / Origin / Destination sind die einzigen Signale stark genug für einen „altes OFP"-Befund. Eine Callsign-Differenz bei sonst stimmiger Route + Aircraft ist fast immer ein legitimer persönlicher ATC-Callsign.
- **Kein Hinweis wenn überhaupt kein OFP an die Buchung gebunden ist.** Vorher rätselte der Pilot warum die Plan-Cards leer sind. Jetzt blauer Info-Banner: „Kein SimBrief-OFP für diese Buchung — erstelle einen auf simbrief.com".

---

## [v0.3.2] — 2026-05-04

Patch-Release direkt nach v0.3.1. Zwei Pilot-Reports vom Live-Test:

### 🐛 Behoben
- **„Discard flight" / „Forget locally" / „Logs löschen" funktionierten auf macOS nicht.** Tauri auf macOS nutzt WKWebView, und WKWebView droppt `window.confirm()` und `window.alert()`-Aufrufe stillschweigend — der Dialog kommt nie, der Aufruf returnt sofort `false`/`undefined`, der Button-Handler springt raus. Auf Windows (WebView2) hat's funktioniert, daher fiel's vorher nicht auf. Alle 6 betroffenen Stellen (`ActiveFlightPanel`, `LandingPanel`, `ActivityLogPanel`, `SettingsPanel`, `ResumeFlightBanner`) nutzen jetzt eine neue In-App-`<ConfirmDialog>`-Component (kein Native-Dialog, kein Plugin, garantiert cross-platform).
- **Loadsheet im Cockpit verglich gegen einen veralteten OFP-Stand.** Real-Pilot-Workflow: Pilot regeneriert auf simbrief.com einen neuen OFP nachdem der Flug schon gestartet ist (Pax/Cargo/Reserve geändert). AeroACARS hatte die Plan-Werte beim Flight-Start eingefroren — der „Refresh"-Button im My-Flights-Tab refreshte nur die Bid-Card-Vorschau, nicht den aktiven Flug-Snapshot. Resultat: Loadsheet zeigte falsche Δ-Werte gegen die Plan-Variante, die der Pilot gar nicht mehr nutzt.

### ✨ Neu
- **OFP-Refresh-Button im Cockpit-Tab** (sichtbar in den Phasen Preflight / Boarding / TaxiOut). Klick → Backend zieht den aktuellen Bid + frische SimBrief-OFP, überschreibt `planned_block` / `planned_tow` / `planned_zfw` / `planned_route` / `planned_alternate` / `max_*` und persistiert sofort. Das Loadsheet vergleicht ab dem Klick gegen den neuen Plan. Activity-Log-Eintrag „OFP refreshed" mit den drei Hauptwerten als Audit-Trail.
- **`<ConfirmDialog>` + `useConfirm()`-Hook** als neue UI-Primitive. Kann von künftigen Components mitgenutzt werden — Esc cancelt, Enter confirmt, Backdrop-Click cancelt, optionaler `destructive`-Mode (rot statt blau). i18n-Keys: `confirm_dialog.default_title` / `confirm` / `cancel`.

### 🛠 Intern
- Neuer Tauri-Command `flight_refresh_simbrief()` — pullt Bid → SimBrief-OFP → mass-assigned planned_*-Felder unter dem `active_flight`-Lock. Verifiziert Bid-ID nach dem Await damit ein parallel-discarded Flight nicht überschrieben wird.

---

## [v0.3.1] — 2026-05-04

Konsolidierter 0.3.x-Release. Bündelt das komplette SimBrief-Integration-Paket (Phase H.7), erweiterte X-Plane-Telemetrie, Live-Block-Fuel-Fix, das Loadsheet-Feature, OFP-Mismatch-Detection, UX-Polish nach dem GSG-Live-Test sowie das **neue Divert-Manual-PIREP-Routing**.

### 🌟 Highlights
- **Divert-Manual-PIREP** — landet jetzt sauber im PENDING-Bucket des VA-Admins statt fälschlich auto-akzeptiert zu werden. Pilot klickt „Divert nach XXX" → PIREP wird als manueller Eintrag mit dem tatsächlichen Landing-Airport für Admin-Review markiert.
- **Loadsheet-Feature** — Live-Anzeige Block-Fuel / ZFW / TOW während Boarding plus Score-Bewertung im Landung-Tab.
- **SimBrief Soll/Ist-Vergleich** — kompletter Plan-vs-Actual-Block im Landung-Tab, farbcodiert mit aviation-tauglichen Schwellen (5/10 %).
- **OFP-Mismatch-Detection** — erkennt wenn der zuletzt von SimBrief geladene OFP nicht zur aktuellen Buchung passt.
- **X-Plane Auto-Reconnect + neue Telemetrie** — startet sich selbst neu, liefert Wing-/Wheel-Well-Lights + TO-Config-Warning für 737 Zibo/LevelUp + universelle Autobrake/XPDR-Labels.

> Hintergrund: v0.3.0 war als Tag bereits gesetzt, aber ohne Release-Notes. Statt rückwirkend zu rekonstruieren bündeln wir alles unter v0.3.1 — alles, was seit v0.2.4 reingegangen ist.

### 🐛 X-Plane Bug-Fixes
- **Gear-DataRef [0]-Index-Fix.** `sim/flightmodel2/gear/deploy_ratio[0]` mit explizitem Index — fixt „Gear UP am Boden" bei LevelUp 737 (gleiches RREF-Pattern wie der Engine-Bug damals).
- **Auto-Reconnect hart abgesichert.** Re-Subscribe-Loop alle 5 s wenn State ≠ Connected. Funktioniert in allen Szenarien: AeroACARS startet vor X-Plane, X-Plane Restart, X-Plane Crash, Aircraft-Wechsel mit Daten-Stillstand.

### ✨ X-Plane Erweiterungen
- **Autobrake-Stufe als Label** (universell, alle Aircraft) — `RTO/OFF/1/2/3/MAX`.
- **XPDR-Mode als Label** (universell) — `OFF/STBY/XPNDR/TEST/ALT/TA/TA-RA`.
- **Wing-Lights** (Boeing 737 Zibo / LevelUp).
- **Wheel-Well-Lights** (737 Zibo / LevelUp).
- **Takeoff-Config-Warning** (737 Zibo / LevelUp) — Warnung im Cockpit-Status wenn Flaps / Trim / Spoiler nicht für TO konfiguriert.

### 📡 phpVMS Live-Display
- **Live-Block-Fuel im `UpdateBody`** wird bei jedem Heartbeat mitgeschickt. phpVMS leitet „Verbleibender Treibstoff = block_fuel − fuel_used" daraus ab; ohne Feld defaultete block_fuel auf 0, Anzeige zeigte „−<fuel_used>" für den ganzen Flug („−17008 kg"-Bug).
- **Bid-Card erweitert** um Aircraft-Type + Marketing-Name + Load-Chips (Pax blau, Cargo orange) + Flight-Type-Badge (PAX/CARGO/CHARTER/REPO). Reihenfolge der Plan-Cards aviation-korrekt: Block → Trip → Reserve | ZFW → TOW → LDW | Alt.
- **SimBrief-Plan-Vorschau** im Briefing per `fetch_simbrief_preview` direkt auf der Bid-Card — Pilot sieht Block / Trip / Reserve / TOW / LDW / ZFW / Alternate VOR dem Tanken, ohne den OFP-Link zu öffnen.

### 🛫 SimBrief-Integration (Phase H.7)
- **API-Client für SimBrief XML-Fetcher** (`xml.fetcher.php`, beide ID-Varianten — numerische SimBrief-ID und Username). Backend-Anbindung läuft automatisch über die phpVMS-Bid-Relation, kein explizites Setup im Settings-Tab nötig.
- **`FlightStats` erweitert** um Plan-Felder: `planned_block_fuel_kg` / `planned_burn_kg` / `planned_reserve_kg` / `planned_zfw_kg` / `planned_tow_kg` / `planned_ldw_kg` / `planned_taxi_kg` / `max_zfw_kg` / `max_tow_kg` / `max_ldw_kg` + Aircraft-Reg + Plan-Route + Plan-Alternate.
- **Landung-Tab** mit komplettem Fuel + Weight + ZFW Soll/Ist/Δ — farbcodiert grün/gelb/rot. Schwellen praxisnah: <5 % grün, 5-10 % gelb, >10 % rot (vorher 1/3 % — viel zu eng für realen Flugbetrieb).
- **Overweight-Warnungen** wenn IST > MAX bei TOW / LDW / ZFW (`LoadsheetMonitor.tsx`).
- **OFP-Mismatch-Detection.** Vergleicht 4 Signale zwischen SimBrief-OFP und phpVMS-Buchung: Aircraft-Type, Origin, Destination, Flight-Number / ATC-Callsign (mit 4 Match-Formaten: direkt, ICAO-Prefix, ATC-Callsign, ATC mit Airline-Prefix). Bei Mismatch werden OFP-Werte komplett ausgeblendet damit keine falschen Daten angezeigt werden — Pilot sieht klaren Banner und weiß: neuen OFP generieren.

### 📋 Loadsheet-Feature (neu in 0.3.x)
- **`LoadsheetMonitor` im Cockpit-Tab** — sichtbar nur in Phase Preflight / Boarding (verschwindet ab TaxiOut). 3 Zeilen mit IST / SOLL / Δ / MAX für Block-Fuel / ZFW / TOW. Inline-Hints: „✓ Bereit für Pushback" / „🛢 Tankvorgang läuft — noch X kg fehlen" / „👥 Boarding läuft — noch X kg fehlen" / „💡 +X kg über Plan".
- **`LoadsheetScore` im Landung-Tab.** Score 0-100 basierend auf Δ% pro Wert (Block/TOW/LDW/ZFW): >5 % = -5 Punkte, >10 % = -15 Punkte. Score-Farbe ≥90 grün, ≥70 gelb, sonst rot. Plus Breakdown-Liste mit ✓/⚠/✕ pro Wert.
- **„Über-Tankt"-Hint im Activity-Log** beim Block-Off-Trigger wenn Block-IST > Plan + Reserve + 500 kg Toleranz. Sanft formuliert („Sehr viel Sprit an Bord, höherer Burn unterwegs zu erwarten") — keine Warnung, nur Cost-Index-Bewusstsein.
- **Loadsheet-Activity-Log @ Block-Off** einmalig „📋 Loadsheet @ Block-off" + Detailzeile (Block / ZFW / TOW). Wandert sowohl in den Cockpit-Activity-Log als auch in den phpVMS-PIREP-ACARS-Log. Dedup über `loadsheet_logged_at_blockoff` Flag (überlebt Resume-after-Crash).

### 🎨 UX-Polish nach GSG-Live-Test
- **Loadsheet im InfoStrip-Stil** (gleiche Optik wie der MASSE/FLUG/TRIP-Strip oben). Keine eigene Box — gehört visuell zum aktiven Flug-Block. Inline-Δ-Suffix statt eigener Spalte: „BLOCK 6.334 kg +0", „TOW 64.544 kg +227". Toggle-Button [▾]/[▸] zum Ein-/Ausklappen.
- **Wetter-Briefing 1-Zeilen-Format** ersetzt die alten 2 Cards: `ABFLUG EDDW 010°/6 kt · 👁 ≥ 10 km · 18°/12° · 1013 hPa  🌦 -SHRA  [▸ METAR]`. METAR-Text aufklappbar. Spart ~200 px Höhe.
- **Wetter-Phänomen-Pills** mit Icon + Code (🌦 SHRA / ⛈ TSRA / ☁ OVC / 🌫 FG) parsed aus dem METAR-Rawtext + Bewölkungs-Fallback.
- **Sicht-Fallback** aus Raw-METAR (`9999` → „≥ 10 km", `CAVOK` → ☀) wenn der Backend-Parser nichts liefert.
- **Visibility-Threshold 9.5 km** statt 10.0 km für die ≥10 km-Anzeige (Aviation-Konvention `9999 m = "10 km oder mehr"`).
- **Cockpit-Tab kompakter:** LiveTapes ~10 % schmaler (Padding 10/14 → 8/12, Schrift 22 → 20 px). RouteMap erst ab Pushback einblenden — vor Pushback ist 0 % Strecke logisch unsinnig.
- **PMDG-Status False-Positive-Fix.** SDK-Warnung wurde fälschlich gefired wenn Sim noch nicht connected, Aircraft im Loading, oder PMDG NG3 in der 20-60s Init-Phase. Jetzt 4-stufiger Check: simState=connected + aircraft_loaded + 20 s Geduld nach Subscribe + ever_received=false.

### ⚙️ Auto-Start-UX
- **Activity-Log-Hint wenn Auto-Start nicht greifen kann.** Drei spezifische Reasons mit jeweils eigener Meldung, throttled 1×/60 s pro Reason: Triebwerke an / Flugzeug rollt / in der Luft.
- **Auto-Start-Skip-Banner im Briefing-Tab.** Gelber Banner mit Begründung im Briefing-Tab — vorher musste der Pilot im Settings-Activity-Log nachschauen oder rätseln warum nichts passiert.
- **Auto-Start-State im Backend persistiert** (`app_config_dir/auto_start.json`). Bisher war `localStorage` die Source of Truth — nach Force-Kill / Hot-Reload im Tauri-Dev-Mode gelegentlich inkonsistent zum Watcher. Frontend zieht beim Mount den Backend-Wert und syncht localStorage als reinen Cache.

### 🛬 Divert-PIREP-Routing (Fix vom 2026-05-04)
- **Diverts werden nicht mehr fälschlich auto-akzeptiert.** phpVMS' `Acars\PirepController::file()` prüft beim Submit nur die Rang-Regel `auto_approve_acars` und ignoriert ein vorher per Smuggle gesetztes `source=MANUAL`. Sobald der PIREP danach `ACCEPTED` ist, blockt `checkReadOnly()` jeden weiteren State-Update — `state→PENDING` schlug mit „PIREP is read-only" fehl.

  **Neuer Pfad:** Bei Divert wird `/file` komplett übersprungen. Stattdessen ein einziger `update_pirep`-Call der `state=PENDING`, `source=MANUAL`, `arr_airport_id`, alle Stats und Timestamps mass-assigned **solange der PIREP noch IN_PROGRESS ist**. Verifiziert gegen phpvms@dev: `PirepController::update` + `parsePirep()` schieben alles per Mass-Assign auf den Pirep-Record, alle nötigen Felder sind in `$fillable`. Der PIREP landet sauber im PENDING-Bucket des VA-Admins ohne Auto-Approve-Trigger.
- **Activity-Log-Display-Fix.** Zeigt bei Divert die echte Arrival-ICAO mit „(DIVERT, planned X)" Suffix statt der alten Plan-Destination — sowohl im Auto-Path als auch im Manual-Path.

### 🛠 Intern
- `UpdateBody` (api-client) erweitert um `arr_airport_id`, `landing_rate`, `score`, `submitted_at`, `block_on_time` für den Divert-Mass-Assign-Pfad.
- `PirepFull.distance` entfernt — phpVMS gibt das Feld inkonsistent als Objekt oder Zahl zurück, wir brauchen's für den State-Check eh nicht.
- `SimSnapshot` erweitert um `light_wing`, `light_wheel_well`, `xpdr_mode_label`, `takeoff_config_warning` als universelle Felder. PMDG-Adapter füllt sie via `snapshot()`-merge, X-Plane-Adapter via DataRefs. Activity-Log liest direkt aus `snap.*` statt aus `snap.pmdg.*` → einheitlicher Pfad.
- Tests: 76 grün (unverändert).

### 📭 Bewusst nicht in 0.3.x
Diese Punkte standen mal auf dem Master-Plan, sind aber nicht enthalten — Code-Verifikation per Grep:
- **Aircraft-Reg-Verifikation (SimBrief vs. Sim).** War in v0.1.x drin, wegen MSFS-2024 Pilot-Profil-Override mit False-Positives wieder ausgebaut. Bleibt skipped bis ein WASM-Livery-Reader steht.
- **Settings-Tab SimBrief-ID/Username-Eingabefeld + Test-Button + Status-Pill.** SimBrief-Anbindung läuft automatisch über die phpVMS-Bid-Relation, daher kein expliziter Setup-Schritt nötig.
- **One-Time Update-Banner im Cockpit-Tab nach erstem Start.** Aus dem gleichen Grund nicht implementiert.
- **„Tipp"-Hinweise im Activity-Log wenn ohne SB-ID gestartet.** Same.

---

## Frühere Versionen

Notes für v0.2.x und v0.1.x liegen in den jeweiligen Release-Commit-Messages (`git log --oneline | grep "release:"`). Die Tags `v0.3.0` (Dev-Build, 2026-05-03) und v0.3.1 markieren denselben funktionalen Release-Zweig — alles, was zwischen v0.2.4 und v0.3.1 reingewachsen ist, steht oben unter `[v0.3.1]`.
