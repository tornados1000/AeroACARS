# Fenix A32x Cockpit-State (Opt-in Beta) - v0.7.16

**Status:** IMPLEMENTED — v0.7.16 Stable mit Opt-in-Beta-Flag  
**Datum:** 2026-05-12  
**Release-Ziel:** `v0.7.16` Stable. Fenix-Profil ist Opt-in, Default off.  
**Scope:** Read-only Cockpit-State fuer Fenix A319/A320/A321

## Kurzfassung

Fenix-Erweiterungen kommen mit `v0.7.16` als **Opt-in Beta-Feature** ins normale Stable-Release:

```text
v0.7.16 Stable - Fenix A32x Cockpit-State Profil (Opt-in, default off)
```

Alle Piloten bekommen v0.7.16 ueber den normalen Auto-Updater. Solange der Beta-Schalter aus ist, ist das Verhalten bit-identisch zu v0.7.15 — auch fuer Fenix-User. Fenix-Tester schalten ihn in den Settings selbst ein und geben Feedback.

Ziel ist: AeroACARS soll bei Fenix A319/A320/A321 Cockpit-Zustaende besser lesen koennen, ohne in das Flugzeug einzugreifen.

## Leitentscheidung

**Read-only zuerst.**

Diese Beta darf:

- Fenix A32x erkennen
- relevante Fenix-LVARs lesen
- bestehende AeroACARS-Cockpit-State-Felder genauer fuellen
- Beta-Logs fuer QS schreiben

Diese Beta darf nicht:

- Schalter setzen
- Knobs drehen
- Fenix-Systeme steuern
- FSUIPC voraussetzen
- normale Piloten blockieren
- einen Flug abbrechen, wenn Fenix-Werte fehlen

Wenn etwas fehlt oder nicht gelesen werden kann, bleibt das Feld `None` oder faellt auf Standard-MSFS-SimVars zurueck.

## Warum Beta

Fenix nutzt viele eigene LVARs und Cockpit-Behaviour-Definitionen. Diese sind leistungsfaehig, koennen sich aber mit Fenix-Updates aendern.

Deshalb:

- kein Stable-Rollout in den Default-Path (= ohne Opt-in)
- keine Pflicht fuer Fenix-Piloten
- Tester schalten den Beta-Flag manuell ein
- erst lesen, dann vergleichen, dann entscheiden, ob die LVAR-Mappings spaeter in den Default-Path uebernommen werden

## Zielgruppe der Beta

Fenix-Piloten mit:

- Fenix A319
- Fenix A320
- Fenix A321
- nach Moeglichkeit MSFS 2020 und MSFS 2024

Tester muessen bereit sein, Activity-Logs/Screenshots zu liefern.

## Technische Quelle

Fenix dokumentiert den Zugriff auf Schalter/Knobs ueber LVARs. Relevante Quelle im installierten Flugzeug:

```text
Community\fnx-aircraft-320\SimObjects\Airplanes\FNX_32X\model\FNX32X_Interior.xml
Community\fnx-aircraft-319-321\SimObjects\Airplanes\FNX_32X\model\FNX32X_Interior.xml
```

Fruehere Fenix-Beispiele nennen teilweise noch Pfade unter:

```text
attachments\fnx\Part_Interior_Cockpit\model\Cockpit_Behavior.xml
```

Das ist fuer die aktuell verifizierte Installation nicht mehr die relevante Struktur. Fuer v0.7.16 gilt die `FNX32X_Interior.xml` im jeweiligen `FNX_32X\model`-Ordner als Primaerquelle.

Dort stehen u.a. `VAR_NAME`-Eintraege, die fuer externe Hardware-/Binding-Tools genutzt werden.

Beispiele aus Fenix-Doku:

```text
S_OH_EXT_LT_BEACON
A_FCU_LIGHTING
A_OH_LIGHTING_OVD
A_PED_LIGHTING_PEDESTAL
```

## Verifizierter Install-Check 2026-05-12

Gegen lokale Fenix-Installation auf:

```text
D:\MSFS Folder\Community\
```

geprueft:

- `fnx-aircraft-320`
- `fnx-aircraft-319-321`

Beide nutzen den gemeinsamen SimObject-Pfad `FNX_32X`. Damit ist die Annahme plausibel, dass die LVAR-Namen fuer A319/A320/A321 variant-uebergreifend identisch sind.

### Bestaetigte LVAR-Kandidaten

| LVAR | Bedeutung | Beta-Status |
|---|---|---|
| `S_OH_EXT_LT_BEACON` | Beacon Light | Pflicht |
| `S_OH_EXT_LT_STROBE` | Strobe Light | Pflicht |
| `S_OH_EXT_LT_NAV_LOGO` | Nav/Logo Light | Pflicht |
| `S_OH_EXT_LT_WING` | Wing Light | Optional |
| `S_OH_EXT_LT_RWY_TURNOFF` | Runway Turnoff Light | Optional |
| `S_OH_EXT_LT_LANDING_L` | Landing Light links | Pflicht |
| `S_OH_EXT_LT_LANDING_R` | Landing Light rechts | Pflicht |
| `S_OH_EXT_LT_LANDING_BOTH` | Landing-Light Composite | Optional / pruefen |
| `S_OH_EXT_LT_NOSE` | Nose/Taxi Light | Pflicht |
| `S_MIP_PARKING_BRAKE` | Parking Brake | Pflicht |
| `S_FC_FLAPS` | Flaps Lever | Optional |

Hinweis zu `S_OH_EXT_LT_LANDING_BOTH`: Diese Variable wirkt wie ein Composite. Fuer Beta erst live pruefen, ob sie synchron zu L/R ist. Stabiler MVP ist L/R einzeln lesen und intern zu `landing_lights_on` kombinieren.

## Scope v0.7.16 (Opt-in Beta)

### Fenix-Erkennung

AeroACARS soll ein Beta-Profil aktivieren, wenn die aktuelle MSFS-Maschine eindeutig Fenix A32x ist.

Moegliche Signale:

- `aircraft_title`
- `aircraft_icao`
- aircraft path / package path, falls verfuegbar
- bekannte Fenix-Prefixe wie `FNX_32X`

Ergebnis:

```text
aircraft_profile = fenix_a32x_beta
```

Wenn nicht eindeutig erkannt:

- Profil bleibt aus
- Standard-MSFS-SimVars laufen weiter
- kein Fehler fuer den Piloten

### LVAR-Discovery

Vor Implementierung muss eine Discovery-Liste erstellt werden:

1. `Cockpit_Behavior.xml` lokal auslesen.
2. Relevante `VAR_NAME`-Eintraege sammeln.
3. Kandidaten nach Cockpit-Bereich gruppieren.
4. Live gegen Fenix A319/A320/A321 pruefen.
5. Nur stabile, verstandene Werte ins Beta-Mapping aufnehmen.

Aktueller Pfad fuer Discovery:

```text
FNX_32X\model\FNX32X_Interior.xml
```

Kein blindes Mapping aller LVARs.

### Read-only Werte

Beta-Kandidaten:

- Beacon
- Landing Lights
- Strobe
- Taxi Light
- Nav Light
- Logo Light, falls vorhanden
- Seatbelt
- No Smoking
- APU Master / APU Running, falls sinnvoll lesbar
- Packs
- Engine Anti-Ice
- Wing Anti-Ice
- Autobrake
- Spoilers Armed
- Parking Brake, falls Fenix-Standardwert genauer ist
- FCU/AP Status nur, wenn eindeutig und stabil

Nicht fuer Beta:

- FMC/MCDU-Daten
- Flightplan-Manipulation
- Performance-Init
- Managed/Selected Mode-Logik, wenn nicht eindeutig
- Writes/Commands
- Payload/Fuel-Setzen

## Architekturvorschlag

### Feature-Flag

Fenix Beta muss schaltbar sein:

```text
fenix_beta_enabled = true/false
```

Default:

```text
false
```

Nur Beta-Tester aktivieren das Profil.

### Adapter-Schicht

Neue interne Schicht:

```text
fenix_a32x_profile
```

Aufgaben:

- erkennen, ob Fenix aktiv ist
- LVARs lesen
- Werte normalisieren
- in bestehende `SimSnapshot`-Felder mappen
- Fehler leise behandeln

### Keine FSUIPC-Abhaengigkeit

Diese Beta darf keine FSUIPC-Pflicht einfuehren.

Moegliche technische Wege muessen separat bewertet werden:

- MSFS/WASM-LVAR-Bridge
- eigener kleiner interner LVAR-Reader
- vorhandene MSFS-Schnittstellen, falls ausreichend

Entscheidungspunkt:

```text
Wie lesen wir LVARs stabil ohne FSUIPC?
```

Das ist der wichtigste technische Spike vor der eigentlichen Implementierung.

## Fehlerverhalten

Bei fehlender Fenix-Installation:

- kein Fehler
- Profil bleibt aus

Bei fehlendem LVAR:

- Feld bleibt `None`
- maximal Debug-Log

Bei teilweiser Verfuegbarkeit:

- vorhandene Werte nutzen
- fehlende Werte ignorieren
- einmalige Activity-/Debug-Info fuer Beta-QS erlaubt:

```text
Fenix beta profile partially available
```

Bei Lese-Fehlern:

- kein Crash
- kein Flugabbruch
- kein Blockieren von PIREP
- Fallback auf Standard-MSFS-SimVars

## Datenmodell

Bestehende `SimSnapshot`-Felder sollen wiederverwendet werden.

Keine neue Webapp-/phpVMS-Pflicht.

Falls noetig optional:

```rust
aircraft_profile: Option<String> // "fenix_a32x_beta"
```

und/oder ein interner Debug-Block:

```rust
fenix_beta_status: Option<FenixBetaStatus>
```

Nur aufnehmen, wenn fuer QS wirklich nuetzlich.

## QS-Plan Beta

### Pflicht-Szenarien

| # | Szenario | Erwartung |
|---|---|---|
| Q1 | Fenix nicht installiert | AeroACARS laeuft normal, kein Fehler |
| Q2 | Fenix installiert, Feature-Flag aus | Standard-MSFS-Verhalten, kein Beta-Profil |
| Q3 | Fenix A319, Flag an | Profil wird erkannt |
| Q4 | Fenix A320, Flag an | Profil wird erkannt |
| Q5 | Fenix A321, Flag an | Profil wird erkannt |
| Q6 | Beacon on/off | AeroACARS erkennt Wechsel |
| Q7 | Landing Lights on/off | AeroACARS erkennt Wechsel |
| Q8 | Strobe/Taxi/Nav | AeroACARS erkennt Wechsel soweit gemappt |
| Q9 | Seatbelt/No Smoking | AeroACARS erkennt Wechsel soweit gemappt |
| Q10 | APU/Packs/Anti-Ice | Werte plausibel oder sauber `None` |
| Q11 | Cold & Dark | keine falschen "on"-Zustaende |
| Q12 | Taxi/Takeoff/Cruise/Approach/Landing | keine Crashs, Werte bleiben plausibel |
| Q13 | Fenix Update / unbekannter LVAR | kein Crash, Fallback |

### Tester-Matrix

| Tester | Sim | Aircraft | Ziel |
|---|---|---|---|
| Tester 1 | MSFS 2020 | Fenix A320 | Hauptpfad |
| Tester 2 | MSFS 2024 | Fenix A320 | MSFS24-Kompat |
| Tester 3 | MSFS 2020/2024 | Fenix A319 | Variantencheck |
| Tester 4 | MSFS 2020/2024 | Fenix A321 | Variantencheck |

## Release-Regeln

`v0.7.16` geht als normales Stable-Release ueber den Auto-Updater an alle Piloten — das Fenix-Profil ist aber Opt-in und Default off, also fuer die breite Nutzerbasis kein Risiko.

Erst nach Beta-QS entscheiden, was mit den LVAR-Mappings im **Default-Path** (= ohne Opt-in) passiert:

- LVAR-Mappings auch ohne Opt-in als Default fuer Fenix-Profile aktiv schalten
- weiter als Opt-in lassen
- einzelne Mappings entfernen
- Architektur ueberarbeiten

## Definition of Done fuer v0.7.16

- Feature-Flag vorhanden, default off
- Fenix-Erkennung funktioniert fuer mindestens A320
- keine FSUIPC-Abhaengigkeit
- keine Writes/Commands
- mindestens Beacon + Landing Lights read-only gemappt
- fehlende LVARs crashen nicht
- `cargo check` gruen
- `cargo test` gruen
- kurzer Beta-QS-Guide fuer Tester vorhanden

## Discord-/Tester-Hinweis Entwurf

```text
Wir testen eine Fenix A32x Beta fuer AeroACARS.

Die Beta liest zunaechst nur Cockpit-Zustaende aus der Fenix A319/A320/A321 aus. Es werden keine Schalter gesetzt und keine Flugzeugsysteme gesteuert.

Ziel ist, AeroACARS-Logs und Cockpit-State-Erkennung fuer Fenix zu verbessern.

Die Beta ist freiwillig und standardmaessig deaktiviert — wer den Schalter in den Settings nicht anklickt, merkt nichts davon. Wenn etwas nicht erkannt wird, bleibt AeroACARS im normalen MSFS-Modus.

Bitte testet Cold & Dark, Taxi, Takeoff, Cruise, Approach und Landing und meldet Abweichungen mit Screenshot/Activity-Log.
```
