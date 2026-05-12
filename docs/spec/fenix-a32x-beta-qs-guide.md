# Fenix A32x Opt-in Beta — QS-Guide fuer Tester (v0.7.16)

**Datum:** 2026-05-12
**Zielgruppe:** Fenix-Piloten mit Fenix A319 / A320 / A321, die das Opt-in-Profil testen wollen
**Begleitende Spec:** [fenix-a32x-cockpit-state-beta.md](fenix-a32x-cockpit-state-beta.md)

---

## Worum es geht

AeroACARS hat in v0.7.15 bereits eine breite Fenix-Anbindung (Lichter,
Parking-Brake, Signs, APU, Anti-Ice, FCU, Autobrake). In `v0.7.16`
kommt eine **additive** Erweiterung als Opt-in-Beta dazu:

- Landing-Lights L/R als 3-Positions-Selektor (retracted / off / on)
- Nose-Light (off / taxi / T.O.)
- Wing-Inspection-Light (off / on)
- Runway-Turnoff-Light (read-only, nur Verifikation)
- `LANDING_BOTH` Composite-LVAR (read-only, nur Verifikation)
- Flaps-Lever-Detent `S_FC_FLAPS` (read-only, nur Verifikation)

Diese Erweiterung steht hinter einem Feature-Flag und ist
**standardmaessig aus**. v0.7.16 geht als normales Stable-Release an alle Piloten — wer den Schalter nicht anklickt, fliegt bit-identisch zu v0.7.15.

---

## Voraussetzungen

- AeroACARS Pilot Client `v0.7.16` (Windows, normales Stable-Update)
- MSFS 2020 oder MSFS 2024
- Fenix A319, A320 oder A321 installiert
- FSUIPC ist **nicht** erforderlich und wird **nicht** verwendet

---

## Aktivierung

Im AeroACARS Settings-Panel → Beta → **Fenix A32x Beta** anschalten.

Hinter den Kulissen ruft das Frontend `set_fenix_beta_enabled(true)`
auf. Das Backend setzt ein Atomic-Flag auf der `MsfsAdapter`-Instanz.
Aenderung wirkt sofort ab dem naechsten Telemetrie-Tick (1 Hz).

Ausschalten: gleiches Panel auf aus. Wirkt sofort, kein Sim-Neustart.

---

## Test-Matrix

### Q1 — Kein Fenix geladen

| Schritt | Erwartung |
|---|---|
| Asobo A320 Neo laden | `aircraft_profile` = `default`, alle Lights aus Standard-SimVars |
| Beta-Flag an oder aus | Verhalten identisch |

### Q2 — Fenix geladen, Beta aus

| Schritt | Erwartung |
|---|---|
| FenixA320 laden | `aircraft_profile` = `fenix_a320` |
| Cold & Dark, Beta aus | Lights wie in v0.7.15 stable — keine Aenderung sichtbar |
| `light_wing` im Snapshot | `None` |

### Q3 — Fenix A320, Beta an

| Schritt | Erwartung |
|---|---|
| FenixA320 laden, Beta an | `aircraft_profile` = `fenix_a320` |
| Beacon ON | `light_beacon` = true (Stable-Pfad, unveraendert) |
| Strobe AUTO / ON | `strobe_state` 1 / 2 (Stable-Pfad, unveraendert) |
| Nose-Light auf "taxi" | `light_taxi` = true |
| Nose-Light auf "T.O." | `light_taxi` = true (gleicher Wert wie taxi) |
| Nose-Light auf "off" | `light_taxi` = false |
| Landing L auf "on" (Pos 2) | `light_landing` = true |
| Landing R auf "on" (Pos 2) | `light_landing` = true |
| Landing L+R retracted (Pos 0) | `light_landing` = false |
| Landing L+R "off" (Pos 1) | `light_landing` = false |
| Wing-Inspection on | `light_wing` = true |
| Wing-Inspection off | `light_wing` = false |

### Q4 — Fenix A319, Beta an

| Schritt | Erwartung |
|---|---|
| FenixA319 laden | `aircraft_profile` = `fenix_a319`, Label "Fenix A319" |
| Lichter wie Q3 | Verhalten identisch zu A320 (gleiche LVARs) |

### Q5 — Fenix A321, Beta an

| Schritt | Erwartung |
|---|---|
| FenixA321 laden | `aircraft_profile` = `fenix_a321`, Label "Fenix A321" |
| Lichter wie Q3 | Verhalten identisch zu A320 (gleiche LVARs) |

### Q6 — `LANDING_BOTH` Konsistenz

| Schritt | Erwartung |
|---|---|
| Im AeroACARS-Inspector `L:S_OH_EXT_LT_LANDING_BOTH` adden | Wert sollte mit `_L` und `_R` synchron sein |
| Pilot drueckt den Both-Switch | `_L`, `_R`, `_BOTH` aendern sich zusammen |

Falls Abweichung beobachtet wird, bitte Screenshot + Activity-Log
melden — dann pruefen wir, ob `_BOTH` doch eigenstaendig getriggert
werden kann.

### Q7 — Phasen-Check

| Phase | Erwartung |
|---|---|
| Cold & Dark | keine falsch-positiven "on"-Zustaende |
| Taxi-Out | Nose=taxi → `light_taxi` true |
| Takeoff-Roll | Nose=T.O., Landing on → beide true |
| Cruise | Landing off → false |
| Approach | Landing on → true |
| Landing | gleiches wie Approach |
| Taxi-In | Nose=taxi, Landing off → taxi true, landing false |

### Q8 — Robustheit

| Szenario | Erwartung |
|---|---|
| Fenix nach Update neu laden | Profil-Erkennung weiter ok, keine Crashs |
| Sim-Pause / -Resume | Werte werden nach Resume wieder sauber geliefert |
| Aircraft mid-flight wechseln (von Fenix zu Asobo) | Profil wechselt zu `default`, keine Geister-Lichter |
| Sim-Restart bei aktivem Flug | AeroACARS Auto-Resume (v0.7.15) greift, Profil wird neu detektiert |

---

## Verifikation im AeroACARS-Inspector

Im Settings-Panel → Debug → Inspector → Add Watch koennen folgende
LVARs einzeln beobachtet werden (alle Number, alle 0/1/2):

- `L:S_OH_EXT_LT_LANDING_L`
- `L:S_OH_EXT_LT_LANDING_R`
- `L:S_OH_EXT_LT_LANDING_BOTH`
- `L:S_OH_EXT_LT_NOSE`
- `L:S_OH_EXT_LT_WING`
- `L:S_OH_EXT_LT_RWY_TURNOFF`
- `L:S_FC_FLAPS`

Erwartung: Werte wechseln synchron zu den Schalterstellungen im
Overhead bzw. Pedestal.

---

## Was ein guter Bug-Report enthaelt

1. Fenix-Variante (A319 / A320 / A321) + Engine (CFM / IAE) + Livery
2. MSFS-Version (2020 / 2024) + SU-Nummer
3. Phase im Flug (Cold & Dark, Taxi, Takeoff, …)
4. AeroACARS-Activity-Log-Ausschnitt rund um die Zeit
5. Optional: Screenshot vom AeroACARS-Inspector mit den LVAR-Werten
6. Erwartetes Verhalten vs. tatsaechliches Verhalten

---

## Was diese Beta absichtlich NICHT macht

- Keine Schalter setzen, keine Knobs drehen, keine Fenix-Systeme steuern
- Keine FMC- / MCDU-Daten lesen
- Keine Flightplan-Manipulation
- Keine Payload- / Fuel-Setzung
- Keine FSUIPC-Abhaengigkeit
- Keine PIREP-Pflichtfelder, die ohne Fenix-Beta nicht gefuellt werden koennten
- Kein Auto-Enable, kein stilles Aktivieren (Pilot muss den Schalter selbst klicken)

Wenn ein Wert nicht erkannt wird oder fehlt: AeroACARS bleibt im
normalen MSFS-Modus, kein Fehler fuer den Piloten.

---

## Definition of Done — diese Beta

- [x] Feature-Flag `fenix_beta_enabled`, default off
- [x] Tauri-Command `set_fenix_beta_enabled` / `get_fenix_beta_enabled`
- [x] AircraftProfile `FenixA319`, `FenixA320`, `FenixA321`
- [x] LVARs Landing-L/R, Nose, Wing, Runway-Turnoff, Landing-Both, Flaps-Lever
- [x] Mapping nur unter `fenix_beta` aktiv
- [x] Stable-Verhalten (Beta aus) bit-identisch zu v0.7.15
- [x] `cargo check` gruen
- [x] `cargo test` gruen
- [ ] Tester-Feedback aus Q1-Q8 ohne Blocker

Erst danach wird ueber Stable-Rollout entschieden.
