# LVar-Discovery via MobiFlight HubHop

**Bookmark für die nächsten Aircraft-Profile-Bauten** — wenn ein neues
MSFS-Addon mit kaputten Standard-SimVars auftaucht und wir einen
`AircraftProfile`-Eintrag bauen müssen, ist HubHop die schnellste Quelle
um die nötigen LVar-Namen zu finden.

## Quick-Lookup-URL

**Web:** https://hubhop.mobiflight.com/presets/ (JavaScript-gerendert,
Bots sehen die Daten nicht)

**API:** https://hubhop-api-mgtm.azure-api.net/api/v1/presets
(öffentlich, kein Auth, ~17 MB JSON dump)

Anonyme curl-Abruf:
```bash
curl -sL "https://hubhop-api-mgtm.azure-api.net/api/v1/presets" \
  -A "Mozilla/5.0" \
  -o /tmp/hubhop.json
```

## Datenstruktur

Jedes Preset hat (Beispiel-Eintrag):
```json
{
  "id": "uuid",
  "path": "FSReborn.PH300E (2024).Engine.Engine 1 Stop",
  "vendor": "FSReborn",
  "aircraft": "PH300E (2024)",
  "system": "Engine",
  "label": "Engine 1 Stop",
  "code": "0 (>L:FSR_300E_ENGINE1_KNOB_POS)",
  "presetType": "Input"
}
```

**Wichtig:** `presetType` ist fast immer `"Input"` (= LVar setzen). Aber
in MSFS sind die LVars **lesbar wenn schreibbar** — wir können die selben
LVar-Namen für unsere Telemetrie-Reads nutzen.

## Filter-Beispiele

Python one-liner um alle LVars für einen Vendor/Aircraft zu extrahieren:

```python
import json, re
presets = json.load(open('/tmp/hubhop.json', encoding='utf-8'))
ph = [p for p in presets if p['vendor']=='FSReborn' and p['aircraft']=='PH300E (2024)']
lvars = sorted({m.group(1) for p in ph for m in re.finditer(r'L:(\w+)', p['code'])})
print('\n'.join(lvars))
```

## Aktuell verfügbare Vendoren (Stand 2026-05)

Aus dem Dump 31,435 Presets von **61 Vendoren / 200+ Aircraft**. Top-Vendoren:

| Vendor | Presets | Aircraft | Notiz für ACARS |
|---|---|---|---|
| Microsoft | 4978 | 19 | Default MSFS |
| IniBuilds | 4341 | 12 | A300, A340-Familie, A350 — schon Profile da |
| PMDG | 3743 | 5 | 737/777 — schon Profile da |
| Fly By Wire | 2820 | 4 | A32NX/A380X — schon Profile da |
| FenixSim | 2263 | 2 | A319/A320/A321 — schon Profile da |
| Asobo | 1883 | 40 | Default MSFS, viele Light-GA |
| TFDi | 1831 | 1 | 717 |
| Black Square | 1423 | 14 | Baron, Bonanza, Caravan (siehe v0.12.10) |
| Aerosoft | 1123 | 2 | A330, CRJ |
| Just Flight | 1113 | 11 | PA-28, RV, Hawk T1 |
| A2A | 687 | 2 | Comanche, J-3 Cub |
| iFly | 636 | 1 | 737 Max |
| FSS | 558 | 3 | Bombardier Global |
| Hype Performance Group | 500 | 2 | H125 Helicopter |
| **FSReborn** | **228** | **3** | **FSR500, PH300E (2024), Sting S4** |
| Flight Sim Labs | 228 | 2 | Concorde, A320X |
| FlightFX | 226 | 3 | Crj |
| Leonardo | 221 | 1 | MD-80 |

## Wann ein FSR-ähnliches "Light-Profile" reicht

FSR PH300E (2024) hat 34 distinct LVars, aber **nutzt Standard-SimVars
für alle Sensoren** (N1/N2, Fuel, Gear, Flaps). Wir brauchen LVars
**nur für Engine-State** weil `GENERAL ENG COMBUSTION` in Cold&Dark
unzuverlässig ist.

→ Wenn ein neues Addon ähnlich gestrickt ist (Custom-LVars nur für
Switches, Standard für Sensoren), reicht ein **minimal Profile** mit
2-3 LVar-Reads.

Wenn Fenix/PMDG-ähnlich (eigene Sim-Engine, Standard-SimVars
unzuverlässig), brauchen wir ein **voll Profile** mit 10-15 LVar-Reads.

## Workflow für neues Aircraft-Profile

1. Curl die HubHop-JSON (oder lokalen Cache nutzen wenn frisch)
2. Filter nach `vendor` + `aircraft` für den Addon
3. Extrahiere die `L:*` Vars aus dem `code`-Feld
4. Im Sim Developer-Mode prüfen welche davon **Reader-Werte** liefern
   (wir wollen lesen, nicht nur schreiben)
5. Im Code: `AircraftProfile`-Enum-Eintrag + `detect()`-Branch +
   `icao_fallback()`-Branch + adapter-spezifische LVar-Reads
6. Unit-Tests mit realen Beispiel-Werten

## Lokaler Cache

Der Dump ist 16,9 MB und ändert sich selten — am besten lokal cachen:
```
E:/temp/hubhop_presets.json
```

Wenn alt (>1 Monat): neu pullen mit dem curl-Befehl oben.

## Begleitende Referenzen

- MobiFlight GitHub: https://github.com/MobiFlight/MobiFlight-Connector
- MSFS SDK Local Variables Doku: https://docs.flightsimulator.com/html/Programming_Tools/Reverse_Polish_Notation.htm#local-variables
- Bestehende Aircraft-Profiles im Code: `client/src-tauri/crates/sim-core/src/lib.rs` (suche `AircraftProfile`)
