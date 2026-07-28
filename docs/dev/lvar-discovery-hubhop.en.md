# LVar Discovery via MobiFlight HubHop

**Bookmark for the next aircraft profile builds** — when a new MSFS add-on
with broken standard SimVars shows up and we need to build an
`AircraftProfile` entry, HubHop is the fastest source for finding the
necessary LVar names.

## Quick lookup URL

**Web:** https://hubhop.mobiflight.com/presets/ (JavaScript-rendered,
bots can't see the data)

**API:** https://hubhop-api-mgtm.azure-api.net/api/v1/presets
(public, no auth, ~17 MB JSON dump)

Anonymous curl fetch:
```bash
curl -sL "https://hubhop-api-mgtm.azure-api.net/api/v1/presets" \
  -A "Mozilla/5.0" \
  -o /tmp/hubhop.json
```

## Data structure

Each preset has (example entry):
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

**Important:** `presetType` is almost always `"Input"` (= setting an LVar). But
in MSFS, LVars are **readable if writable** — we can use the same LVar names
for our telemetry reads.

## Filter examples

Python one-liner to extract all LVars for a vendor/aircraft:

```python
import json, re
presets = json.load(open('/tmp/hubhop.json', encoding='utf-8'))
ph = [p for p in presets if p['vendor']=='FSReborn' and p['aircraft']=='PH300E (2024)']
lvars = sorted({m.group(1) for p in ph for m in re.finditer(r'L:(\w+)', p['code'])})
print('\n'.join(lvars))
```

## Currently available vendors (as of 2026-05)

From the dump: 31,435 presets from **61 vendors / 200+ aircraft**. Top vendors:

| Vendor | Presets | Aircraft | Note for ACARS |
|---|---|---|---|
| Microsoft | 4978 | 19 | Default MSFS |
| IniBuilds | 4341 | 12 | A300, A340 family, A350 — profiles already exist |
| PMDG | 3743 | 5 | 737/777 — profiles already exist |
| Fly By Wire | 2820 | 4 | A32NX/A380X — profiles already exist |
| FenixSim | 2263 | 2 | A319/A320/A321 — profiles already exist |
| Asobo | 1883 | 40 | Default MSFS, many light GA |
| TFDi | 1831 | 1 | 717 |
| Black Square | 1423 | 14 | Baron, Bonanza, Caravan (see v0.12.10) |
| Aerosoft | 1123 | 2 | A330, CRJ |
| Just Flight | 1113 | 11 | PA-28, RV, Hawk T1 |
| A2A | 687 | 2 | Comanche, J-3 Cub |
| iFly | 636 | 1 | 737 Max |
| FSS | 558 | 3 | Bombardier Global |
| Hype Performance Group | 500 | 2 | H125 Helicopter |
| **FSReborn** | **228** | **3** | **FSR500, PH300E (2024), Sting S4** |
| Flight Sim Labs | 228 | 2 | Concorde, A320X |
| FlightFX | 226 | 3 | CRJ |
| Leonardo | 221 | 1 | MD-80 |

## When an FSR-like "light profile" is enough

FSR PH300E (2024) has 34 distinct LVars, but **uses standard SimVars
for all sensors** (N1/N2, fuel, gear, flaps). We need LVars
**only for engine state** because `GENERAL ENG COMBUSTION` in cold & dark
is unreliable.

→ If a new add-on is similarly constructed (custom LVars only for
switches, standard for sensors), a **minimal profile** with
2–3 LVar reads is enough.

If it's Fenix/PMDG-like (own sim engine, standard SimVars
unreliable), we need a **full profile** with 10–15 LVar reads.

## Workflow for a new aircraft profile

1. Curl the HubHop JSON (or use the local cache if fresh)
2. Filter by `vendor` + `aircraft` for the add-on
3. Extract the `L:*` vars from the `code` field
4. In the sim developer mode, check which of them provide **reader values**
   (we want to read, not just write)
5. In the code: `AircraftProfile` enum entry + `detect()` branch +
   `icao_fallback()` branch + adapter-specific LVar reads
6. Unit tests with real example values

## Local cache

The dump is 16.9 MB and changes infrequently — best to cache locally:
```
E:/temp/hubhop_presets.json
```

If stale (>1 month): re-pull with the curl command above.

## Related references

- MobiFlight GitHub: https://github.com/MobiFlight/MobiFlight-Connector
- MSFS SDK Local Variables docs: https://docs.flightsimulator.com/html/Programming_Tools/Reverse_Polish_Notation.htm#local-variables
- Existing aircraft profiles in the code: `client/src-tauri/crates/sim-core/src/lib.rs` (search for `AircraftProfile`)
