# X-Plane — Aircraft-spezifische DataRef-Overrides (Vorgabe)

**Zweck:** AeroACARS liest seine X-Plane-Telemetrie über Standard-`sim/...`-DataRefs.
Bei **Study-Level-Add-ons** (Hot-Start Challenger 650, ToLiss, FlightFactor, PMDG …)
laufen viele Cockpit-/System-Funktionen über **eigene** DataRefs des Add-ons — die
Standard-DataRef bleibt dann auf `0` / leer. Genau so entstand der GSG225-Bug:
der CL650 bedient `sim/flightmodel2/controls/flap_handle_deploy_ratio` nicht →
AeroACARS „sah" die Flaps nie → `LANDING CONFIG: INCOMPLETE`.

**Diese Datei ist die Vorlage**, um für ein konkretes Flugzeug die passenden
Add-on-eigenen DataRefs zu finden und nachzutragen. Pro Flugzeug einmal ausfüllen,
dann kann AeroACARS ein Aircraft-Profil dafür bekommen.

---

## So füllst du die Vorgabe aus

1. **DataRefTool installieren** (X-Plane-Plugin, kostenlos): kopiere den Ordner
   `DataRefTool` nach `X-Plane/Resources/plugins/`.
2. Flugzeug laden, ans Gate stellen, **Plugins → DataRefTool → Show DataRefs**.
3. Im Suchfeld den **Add-on-Prefix** eingeben — meist der Hersteller-Namespace:
   - Hot-Start Challenger 650 → `CL650/`
   - ToLiss → `AirbusFBW/`
   - FlightFactor → `1-sim/` bzw. `a350/` / `757/` …
   - PMDG (falls X-Plane-Variante) → herstellerspezifisch
4. Für jede Zeile in der Tabelle unten: im Sim die Funktion **betätigen**
   (z.B. Flaps fahren) und im DataRefTool schauen, **welcher Add-on-DataRef
   sich mitbewegt**. Den Namen + den beobachteten Wertebereich eintragen.
5. Die ausgefüllte Tabelle an die Entwicklung geben → Aircraft-Profil.

> **Tipp:** Erst die Standard-`sim/...`-DataRef beobachten. Bewegt sie sich
> beim Betätigen mit, ist **kein Override nötig** — dann das Feld
> „Aircraft-spezifisch" leer lassen / mit „— (Standard ok)" markieren.

---

## Teil 1 — Physik / Flugmodell: **KEIN Override nötig**

Diese Werte liefert die X-Plane-Flugmodell-Engine immer korrekt, unabhängig
vom Add-on. Hier nie etwas suchen.

Position, Höhe (MSL/AGL), Heading, Pitch/Bank, Vertical Speed, Groundspeed,
IAS/TAS, G-Force, On-Ground, Gear-Normal-Force, Gewicht (leer/total),
Treibstoffmenge, Wind, Mach, OAT, QNH — sowie die **Body-Velocity**
(`sim/flightmodel/forces/local_vx` / `local_vz`), die AeroACARS für die
Sideslip- und Touchdown-Auswertung nutzt. Alles Flugmodell-Werte: jedes
Add-on treibt sie korrekt, hier nie etwas suchen.

---

## Teil 2 — Cockpit / Systeme: **Override-Kandidaten**

Diese Funktionen werden von Study-Level-Add-ons häufig über eigene DataRefs
gesteuert. Pro Flugzeug prüfen und ggf. eintragen.

**Priorität A — fließt in Score / Phasen-Erkennung (zuerst suchen!):**

| Funktion | Standard-DataRef (AeroACARS heute) | Typ / erwarteter Wert | Wirkung in AeroACARS | Aircraft-spezifischer DataRef |
|---|---|---|---|---|
| Flaps-Stellung | `sim/flightmodel2/controls/flap_handle_deploy_ratio` | float 0.0–1.0 | Approach-Stability „Landing Config" | `____________________` |
| Gear-Stellung | `sim/flightmodel2/gear/deploy_ratio[0]` | float 0.0–1.0 | Approach-Stability, Phasen | `____________________` |
| Triebwerk 1 läuft | `sim/flightmodel/engine/ENGN_running[0]` | int 0/1 | Phasen-FSM (Pushback/Taxi/Start) | `____________________` |
| Triebwerk 2 läuft | `sim/flightmodel/engine/ENGN_running[1]` | int 0/1 | Phasen-FSM | `____________________` |
| Triebwerk 3 läuft | `sim/flightmodel/engine/ENGN_running[2]` | int 0/1 | Phasen-FSM (3+ Mot.) | `____________________` |
| Triebwerk 4 läuft | `sim/flightmodel/engine/ENGN_running[3]` | int 0/1 | Phasen-FSM (4 Mot.) | `____________________` |
| Parkbremse | `sim/cockpit2/controls/parking_brake_ratio` | float 0.0–1.0 | Phasen-/Block-Logik | `____________________` |

**Priorität B — PIREP-Custom-Fields / Anzeige (kosmetisch, danach suchen):**

| Funktion | Standard-DataRef (AeroACARS heute) | Typ / erwarteter Wert | Aircraft-spezifischer DataRef |
|---|---|---|---|
| Speedbrake / Spoiler | `sim/cockpit2/controls/speedbrake_ratio` | float 0.0–1.0 | `____________________` |
| Spoiler armed | `sim/cockpit2/annunciators/speedbrake` | int 0/1 | `____________________` |
| Autobrake-Stufe | `sim/cockpit2/switches/auto_brake_level` | int | `____________________` |
| Stall-Warnung | `sim/cockpit2/annunciators/stall_warning` | int 0/1 | `____________________` |
| Landing-Lights | `sim/cockpit2/switches/landing_lights_on` | int 0/1 | `____________________` |
| Beacon | `sim/cockpit2/switches/beacon_on` | int 0/1 | `____________________` |
| Strobe | `sim/cockpit2/switches/strobe_lights_on` | int 0/1 | `____________________` |
| Taxi-Light | `sim/cockpit2/switches/taxi_light_on` | int 0/1 | `____________________` |
| Nav-Lights | `sim/cockpit2/switches/navigation_lights_on` | int 0/1 | `____________________` |
| Wing-Light | `laminar/B738/toggle_switch/wing_light_pos` *(bereits 737-spezifisch)* | int | `____________________` |
| Wheel-Well-Light | `laminar/B738/toggle_switch/wheel_well_light_pos` *(bereits 737-spezifisch)* | int | `____________________` |
| Autopilot Master | `sim/cockpit2/autopilot/servos_on` | int 0/1 | `____________________` |
| AP Heading-Mode | `sim/cockpit2/autopilot/heading_status` | int 0/1/2 | `____________________` |
| AP Altitude-Mode | `sim/cockpit2/autopilot/altitude_hold_status` | int 0/1/2 | `____________________` |
| AP Nav-Mode | `sim/cockpit2/autopilot/nav_status` | int 0/1/2 | `____________________` |
| AP Approach-Mode | `sim/cockpit2/autopilot/approach_status` | int 0/1/2 | `____________________` |
| Battery-Master | `sim/cockpit2/electrical/battery_on[0]` | int 0/1 | `____________________` |
| Avionics-Master | `sim/cockpit2/electrical/avionics_on` | int 0/1 | `____________________` |
| APU | `sim/cockpit2/electrical/APU_running` | int 0/1 | `____________________` |
| Pitot-Heat | `sim/cockpit2/ice/ice_pitot_heat_on_pilot` | int 0/1 | `____________________` |
| Transponder-Mode | `sim/cockpit2/radios/actuators/transponder_mode` | int | `____________________` |
| Takeoff-Config-Warnung | `laminar/B738/annunciator/takeoff_config` *(bereits 737-spezifisch)* | int 0/1 | `____________________` |

---

## Teil 3 — Beispiel: Hot-Start Challenger 650 (X-Plane), Flug GSG225

Bekannter Befund: `flap_handle_deploy_ratio` bleibt `0`, obwohl die Flaps voll
gesetzt sind. AeroACARS behandelt das ab v0.12.1 fail-soft (LANDING CONFIG =
„nicht bewertbar" statt rotem „INCOMPLETE", kein Punktabzug). Mit dem
korrekten CL650-DataRef könnte die Landing-Config wieder echt bewertet werden.

| Funktion | Standard-DataRef | CL650-spezifisch (im DataRefTool unter `CL650/` suchen) |
|---|---|---|
| Flaps-Stellung | `sim/flightmodel2/controls/flap_handle_deploy_ratio` | `____________________` ← **bitte nachtragen** |
| Gear-Stellung | `sim/flightmodel2/gear/deploy_ratio[0]` | `____________________` |
| Triebwerk 1/2 läuft | `ENGN_running[0]` / `[1]` | `____________________` |
| Parkbremse | `parking_brake_ratio` | `____________________` |
| (weitere nach Bedarf — Tabelle Teil 2) | | |

---

## Hinweise

- **Wertebereich angeben:** ein Add-on-DataRef kann anders skaliert sein
  (z.B. Flaps 0–30 statt 0.0–1.0, oder ein Detent-Index 0–4). Bitte den
  beobachteten Bereich mit notieren — das Aircraft-Profil rechnet dann um.
- **Arrays:** manche DataRefs sind Arrays (`[0]`, `[1]` …). Index mit angeben.
- Bewegt sich die Standard-`sim/...`-DataRef korrekt mit → kein Override nötig,
  Feld leer lassen.
- Diese Vorgabe deckt nur X-Plane ab. MSFS-Study-Level (Fenix, PMDG) läuft
  über ein separates SimVar-/LVar-Profil-System.
