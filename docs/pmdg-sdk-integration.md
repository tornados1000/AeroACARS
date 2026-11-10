# PMDG SDK Integration — Implementation Plan

**Status**: Untersuchung abgeschlossen, bereit für Phase 1 Implementation
**Branch**: `feat/pmdg-ng3-sdk`
**Target Release**: v0.2.0 — "Boeing Premium Telemetry"
**Author**: AeroACARS Team
**Date**: 2026-05-03

---

## 1. Why — Was bringt's?

PMDG-Aircraft (737 NG3, 777X) haben einen offiziellen SimConnect-SDK der **Cockpit-State direkt aus dem Flugzeug** liefert. Standard-MSFS-SimVars exposen das nicht — kein anderer phpVMS-ACARS-Client (vmsACARS, SmartCARS, Volanta) nutzt diese Daten.

Was wir damit machen können:
- **Echte FMA-Modes** (TOGA / N1 / SPD / ARM / etc.) live im Activity Log
- **MCP-Settings** (Selected SPD/HDG/ALT/VS) direkt aus dem MCP statt aus FCU-SimVars zurückgerechnet
- **V-Speeds** (V1/VR/V2/VREF) im PIREP — vom FMC berechnet, nicht geraten
- **FMC-Werte** (Cruise-Alt, Distance to TOD, Distance to Dest, Flight Number)
- **Echte Flap-Position in Grad** (`MAIN_TEFlapsNeedle[2]`) statt Detent-Quantisierung
- **Aircraft-Variante** (737-700/-800/-900/etc.) genauer als unser `aircraft_profile`
- **Autobrake-Setting** (RTO/OFF/1/2/3/MAX)
- **TO-Config-Warning** im Activity Log

Story für die User:
> AeroACARS v0.2.0 — Boeing Premium Telemetry
> Direkte SDK-Integration für PMDG 737 + 777. FMA, MCP, V-Speeds, FMC-Daten — alles live im Cockpit-Tab und im PIREP. Erkennt automatisch ob du PMDG fliegst.

---

## 2. Architektur

```
┌─────────────────────────────────────────────────────────────────┐
│                        sim-msfs Crate                           │
│                                                                 │
│  ┌─────────────────────┐       ┌─────────────────────┐         │
│  │  Standard Telemetry │       │   PMDG ClientData   │         │
│  │  (existing)         │       │   (new)             │         │
│  │                     │       │                     │         │
│  │  SimConnect Data    │       │  SimConnect Client  │         │
│  │  Definition #1      │       │  Data #2            │         │
│  │  → Telemetry struct │       │  → PMDG_NG3_Data    │         │
│  │                     │       │     OR              │         │
│  │  All aircraft       │       │  → PMDG_777X_Data   │         │
│  │  (Asobo, FBW,       │       │                     │         │
│  │   Fenix, PMDG, INI) │       │  Only when PMDG     │         │
│  │                     │       │  loaded + SDK on    │         │
│  └─────────────────────┘       └─────────────────────┘         │
│           │                              │                      │
│           ▼                              ▼                      │
│      SimSnapshot.* ◄──── merge ──── PmdgSnapshot                │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                          │
                          ▼
                    AeroACARS App
                  (Cockpit, PIREP, Activity Log)
```

Beide Subscriptions laufen **parallel**. Wenn der Pilot ein non-PMDG-Aircraft fliegt, läuft nur die Standard-Telemetry. Wenn PMDG: zusätzlich PMDG-Daten.

---

## 3. Aircraft-Detection-Flow

```
1. SimConnect_Open() bestätigt Connection
2. SimConnect_RequestSystemState("AircraftLoaded") → liefert .air-Datei-Pfad
3. Pfad-Match:
   - Enthält "PMDG 737" / "pmdg-aircraft-738" → NG3-Modus
   - Enthält "PMDG 777" / "pmdg-aircraft-77er/77w/77f/77l" → 777X-Modus
   - Sonst → Standard-only
4. Bei PMDG-Match:
   a. SimConnect_MapClientDataNameToID(PMDG_NG3_DATA_NAME, ID)
   b. SimConnect_AddToClientDataDefinition(DEFINITION_ID, 0, sizeof(struct), 0, 0)
   c. SimConnect_RequestClientData(... PERIOD_ON_SET, FLAG_CHANGED ...)
5. Subscribe auf "AircraftLoaded"-Changes (SubscribeToSystemEvent "SimStart")
   → bei Aircraft-Wechsel: Cleanup + neu detecten
```

---

## 4. SDK aktivieren (User-Workflow)

PMDG sendet die ClientData NICHT standardmäßig. Pilot muss in der Aircraft-Options-Datei eine Zeile ergänzen:

**Für 737:**
```ini
# E:\MSFS24_Community\Community\pmdg-aircraft-738\work\737NG3_Options.ini
[SDK]
EnableDataBroadcast=1
```

**Für 777:**
```ini
# E:\MSFS24_Community\Community\pmdg-aircraft-77er\work\777X_Options.ini
[SDK]
EnableDataBroadcast=1
```

**UI-Integration:**
- AeroACARS detected PMDG geladen + ClientData kommt nicht (Subscribe meldet keine Daten innerhalb 5s)
- → Settings-Tab zeigt einen orangenen Hinweis: "PMDG SDK nicht aktiviert. Klicke hier für Anleitung."
- Klick öffnet einen Modal mit Schritt-für-Schritt-Anleitung + Pfad-zu-Datei (Auto-Detect)
- Optional: "Datei automatisch öffnen"-Button (öffnet die `.ini`-Datei in Notepad)

---

## 5. Implementation Steps

Sortiert nach Risiko & Abhängigkeit:

### Phase 5.1 — Rust-Replikation der Headers (low risk)

**Ziel:** Beide Headers in Rust replizieren mit `#[repr(C)]` für korrektes Memory-Layout.

**Was:**
- Neuer Modul `crates/sim-msfs/src/pmdg/mod.rs`
- Sub-Module: `pmdg/ng3.rs` und `pmdg/x777.rs`
- Jedes Modul definiert seine `Pmdg{Variant}Data` struct
- Hilfs-Funktion: `pub fn from_bytes(bytes: &[u8]) -> Option<Self>`

**Field-Mapping:**
| C++ | Rust |
|---|---|
| `bool` | `u8` (C++ bool ist 1 byte) |
| `char` | `u8` |
| `unsigned char` | `u8` |
| `short` | `i16` |
| `unsigned short` | `u16` |
| `int` | `i32` |
| `unsigned int` | `u32` |
| `float` | `f32` |
| `char[N]` | `[u8; N]` |

**Critical:** `#[repr(C)]` verwendet — nicht `packed`. Das matched MSVC-Layout (Standard für PMDG-Build).

**Tests:**
- `assert_eq!(std::mem::size_of::<PmdgNg3Data>(), 7000)` (= sizeof aus dem Header)
- Snapshot-Test mit hexed bytes aus einem realen Capture

### Phase 5.2 — SimConnect ClientData-Subscription (medium risk)

**Ziel:** sim-msfs adapter erweitern um optional eine zweite ClientData-Subscription für PMDG-Daten.

**Was:**
- Neues Feld in `Shared`: `pmdg_data: Mutex<Option<PmdgSnapshot>>` (PmdgSnapshot ist ein enum mit `Ng3(...)` und `X777(...)` Varianten)
- Neue Connection-Methode: `Connection::register_pmdg_clientdata(variant: PmdgVariant)` — mappt Name + Definition + Request
- Im `run_dispatch`: extra Branch für `SIMCONNECT_RECV_ID_CLIENT_DATA` mit dem PMDG-Request-ID
- Bei PMDG-Daten: bytes → struct → in `shared.pmdg_data`

**Edge Cases:**
- SimConnect-Exception wenn ClientData-Name nicht registriert ist (= SDK nicht aktiviert) → wir kriegen `EXCEPTION_DATA_ERROR`
- → User-Friendly: state als `PmdgSdkState::SdkNotEnabled` setzen, UI zeigt Hint

### Phase 5.3 — Aircraft-Auto-Detection (low risk)

**Ziel:** Bei Aircraft-Wechsel automatisch PMDG erkennen + Subscription anpassen.

**Was:**
- `SimConnect_RequestSystemState("AircraftLoaded")` direkt nach Connection-Open
- Plus `SubscribeToSystemEvent("SimStart")` für Live-Wechsel
- Pattern-Match auf den `.air`-Pfad:
  ```rust
  fn detect_pmdg_variant(air_path: &str) -> Option<PmdgVariant> {
      let p = air_path.to_lowercase();
      if p.contains("pmdg-aircraft-737") || p.contains("pmdg 737") {
          Some(PmdgVariant::Ng3)
      } else if p.contains("pmdg-aircraft-77") || p.contains("pmdg 777") {
          Some(PmdgVariant::X777)
      } else {
          None
      }
  }
  ```
- Bei Variant-Change: alte Subscription cleanen, neue starten

### Phase 5.4 — SimSnapshot-Integration (low risk)

**Ziel:** PMDG-Daten in `SimSnapshot` einfließen lassen, ohne Standard-Felder zu duplizieren.

**Was:**
- `SimSnapshot` bekommt ein neues Feld:
  ```rust
  pub pmdg: Option<PmdgSnapshot>,
  ```
- `PmdgSnapshot` ist enum mit den Variants `Ng3(Ng3Snapshot)` und `X777(X777Snapshot)`
- `Ng3Snapshot` / `X777Snapshot` sind die "interessanten" Felder gefiltert + in lesbarer Form (z.B. `mcp_speed_kt`, `mcp_speed_is_mach`, `fma_speed_mode: FmaMode` etc.)
- Adapter-Code: `to_snapshot()` merged sowohl Standard-Telemetry als auch PMDG-Daten

**Welche Felder ins Snapshot:**

| Aus PMDG_NG3_Data | Snapshot-Feld | Use Case |
|---|---|---|
| `MCP_IASMach`, `MCP_IASBlank` | `mcp_speed_kt: Option<f32>` | "MCP set to 250 kt" |
| `MCP_Heading` | `mcp_heading_deg: Option<u16>` | "MCP HDG: 280°" |
| `MCP_Altitude` | `mcp_altitude_ft: Option<u16>` | "MCP ALT: 28000 ft" |
| `MCP_VertSpeed`, `MCP_VertSpeedBlank` | `mcp_vs_fpm: Option<i16>` | "MCP V/S: -1500 fpm" |
| `MCP_annunVNAV/LNAV/LVL_CHG/HDG_SEL/...` | `fma_active_modes: FmaModes` | A/T+A/P FMA |
| `FMC_TakeoffFlaps` | `fmc_takeoff_flaps_deg: Option<u8>` | "Plan TO Flaps: 5°" |
| `FMC_LandingFlaps` | `fmc_landing_flaps_deg: Option<u8>` | "Plan LDG Flaps: 30°" |
| `FMC_V1/VR/V2/LandingVREF` | `fmc_v_speeds: VSpeeds` | PIREP-Custom-Field |
| `FMC_CruiseAlt` | `fmc_cruise_alt_ft: Option<u16>` | "FMC Cruise: FL280" |
| `FMC_DistanceToTOD` | `fmc_distance_to_tod_nm: Option<f32>` | TOD-Indicator |
| `FMC_DistanceToDest` | `fmc_distance_to_dest_nm: Option<f32>` | Cross-check |
| `MAIN_TEFlapsNeedle[0]` | `flaps_position_deg: Option<f32>` | Echte Flap-Grad |
| `MAIN_AutobrakeSelector` | `autobrake: Option<AutobrakeSetting>` | "Autobrake MAX" |
| `AircraftModel` | `aircraft_subvariant: Option<u16>` | "B737-800 SSW" |
| `WeightInKg` | `weight_unit_kg: Option<bool>` | Korrekte Einheit |

**FmaMode** als enum:
```rust
pub enum FmaMode {
    Inactive,
    Vnav, Lnav,
    HdgSel, HdgHold,
    LvlChg, AltHold,
    VorLoc, App, Toga, Speed,
    N1, At, Cws,
    // ... etc, je nach annun-Boolean welche aktiv
}
```

### Phase 5.5 — Activity-Log-Integration (medium risk)

**Ziel:** PMDG-Events ins ACARS-Activity-Log loggen, mit Dedup.

**Beispiel-Logs:**
- `MCP IAS → 230 kt` (wenn MCP_IASMach geändert)
- `MCP HDG → 080°` (wenn MCP_Heading geändert)
- `A/T armed` (wenn MCP_annunATArm: false→true)
- `A/P CMD A engaged` (wenn MCP_annunCMD_A: false→true)
- `FMA: VNAV PTH / SPEED / —` (wenn FMA-Modes ändern)
- `Autobrake → MAX` (wenn MAIN_AutobrakeSelector ändert)
- `V1: 145 kt · VR: 148 kt · V2: 152 kt` (einmal beim Takeoff-Roll-Start)
- `TO Config Warning` (wenn MAIN_annunTAKEOFF_CONFIG: true)

**Dedup:** `last_logged_*`-Felder im FlightStats wie heute schon. Trigger: "Wert hat sich geändert" + "stabil für ≥1 Tick". Verhindert Flackerlog.

### Phase 5.6 — PIREP-Custom-Fields (low risk)

**Was kommt rein in den finalen PIREP:**

| Custom Field | Quelle | Beispiel |
|---|---|---|
| "V1 / VR / V2" | `fmc_v_speeds` (zum Takeoff-Zeitpunkt) | "V1 145 / VR 148 / V2 152" |
| "VREF" | `fmc_v_speeds.vref` (zum Landing-Zeitpunkt) | "138 kt" |
| "FMC Cruise Alt" | `fmc_cruise_alt_ft` (Plan-Wert) | "FL280" |
| "TO Flaps Plan" | `fmc_takeoff_flaps_deg` | "5°" |
| "TO Flaps Actual" | `flaps_position_deg` (aus dem Takeoff-Sample) | "5°" |
| "LDG Flaps Plan" | `fmc_landing_flaps_deg` | "30°" |
| "LDG Flaps Actual" | `flaps_position_deg` (aus dem Touchdown-Sample) | "30°" |
| "Autobrake at Land" | `autobrake` (zum Landing-Zeitpunkt) | "MAX" |
| "Aircraft Variant" | `aircraft_subvariant` (NG3) oder Path (777X) | "B737-800 SSW" |

### Phase 5.7 — UI: SDK-Status-Anzeige (medium risk)

**Wo:**
- Settings → Debug-Panel: neue Section "PMDG SDK"
- Cockpit-Tab: kleines Logo / Indicator wenn PMDG-SDK aktiv

**States:**
- `Inactive` — kein PMDG-Aircraft geladen
- `PmdgDetected` — PMDG erkannt, ClientData wird abonniert
- `Active` — Daten kommen (= SDK ist enabled)
- `SdkNotEnabled` — PMDG erkannt, aber keine Daten → Hinweis-Modal

**Modal-Anleitung (DE+EN):**
> Du fliegst eine PMDG 737/777, aber AeroACARS bekommt keine erweiterten Cockpit-Daten. Aktiviere den PMDG SDK:
>
> 1. Schließe MSFS
> 2. Öffne diese Datei: `<auto-detected path>\737NG3_Options.ini`
> 3. Füge am Ende ein:
>    ```
>    [SDK]
>    EnableDataBroadcast=1
>    ```
> 4. Speichern → MSFS neustarten → Flug neu laden
>
> [Datei in Notepad öffnen]   [Anleitung verstanden, später]

---

## 6. Edge Cases

| Fall | Verhalten |
|---|---|
| PMDG nicht installiert | Standard-Telemetry only, kein UI-Hint |
| PMDG installiert aber andere Aircraft geladen | Standard-Telemetry only |
| PMDG geladen, SDK nicht enabled | UI zeigt Hint-Modal, Standard-Telemetry läuft |
| Pilot wechselt mid-flight von PMDG zu Asobo | PMDG-Subscription cleanen, keine pmdg-Daten mehr im Snapshot |
| Pilot wechselt von Asobo zu PMDG | PMDG-Subscription neu aufbauen |
| PMDG-Update ändert Struct-Layout | Wir kriegen falsche Daten → Test mit `assert size_of`. Wenn Mismatch: Log-Warning + fallback to Standard-only |
| SimConnect-Verbindung droppt | Beide Subscriptions cleanen, beim Reconnect neu aufbauen |

---

## 7. Tests

**Unit-Tests:**
- `size_of::<PmdgNg3Data>()` matches expected (catch struct-layout-changes)
- `size_of::<Pmdg777XData>()` matches expected
- Bytes-roundtrip: serialize known-state struct → parse back → equal
- `detect_pmdg_variant()` matches alle bekannten Aircraft-Pfade (PMDG 737, PMDG 777ER, 777W, 777F, 777L)
- FmaMode-Decoder: alle Combinationen von annun-Booleans → korrekte FmaMode

**Integration-Tests (manuell):**
- DEV-Build mit MSFS + PMDG 737 + SDK enabled → MCP-Werte im Debug-Panel
- MCP-Wert ändern im Sim → Activity-Log-Entry erscheint
- Aircraft-Wechsel im Sim → State updates korrekt
- SDK NICHT enabled → Hint-Modal taucht auf nach 5s

---

## 8. Release-Strategie

**Stufenweise Release im feature-branch, kein direkter merge nach main:**

1. **Phase 5.1 + 5.2 + 5.3** (Rust-Struct + Subscription + Detection) → DEV-Build → Test mit User
2. **Phase 5.4** (SimSnapshot-Integration) → DEV-Build → Test
3. **Phase 5.5 + 5.6** (Activity-Log + PIREP-Felder) → DEV-Build → Test mit echtem Flug
4. **Phase 5.7** (UI) → final polish
5. **Merge nach main** → Tag **v0.2.0** → bilinguale Release-Notes:
   > AeroACARS v0.2.0 — Boeing Premium Telemetry
   > PMDG-737/777-SDK-Integration. Echte FMA, MCP, V-Speeds, FMC-Daten — direkt aus dem Cockpit.

**Keine kleinen Hotfix-Releases dazwischen.** Wenn was im Master nicht passt, wird's separat gepatched.

---

## 9. Was wir HEUTE NICHT machen

- ❌ Control-Channel (Output zu PMDG zurück) — wir LESEN nur, schicken keine Befehle
- ❌ CDU-Display lesen (`PMDG_NG3_CDU_0`) — interessant aber separates Feature, vermutlich Phase H.5
- ❌ EFB-Daten (777X) — separates Feature
- ❌ Andere PMDG-Aircraft (DC-6, etc.) — kommt wenn nachgefragt
- ❌ FSUIPC- oder MobiFlight-Bridge — wir nutzen nur den nativen PMDG-SDK

---

## 10. Lizenz & Rechtliches

PMDG erlaubt SDK-Nutzung explizit ([Forum-Bestätigung](https://forum.pmdg.com/forum/main-forum/pmdg-737-for-msfs/general-discussion-no-support/234073)). Der SDK-Header ist als End-User-Resource ausgeliefert (kein NDA, kein Login required, im Plain-Sight in der Aircraft-Installation).

Wir replizieren die Struktur in Rust für eigenen Use — das ist **fair use** und Standard-Praxis (siehe SPAD.neXt, MobiFlight, FSUIPC, alle die das genauso machen).

Im README erwähnen wir:
> AeroACARS unterstützt die PMDG SimConnect-SDK für 737 NG3 und 777X. PMDG ist ein eingetragenes Markenzeichen von Precision Manuals Development Group; AeroACARS ist nicht von PMDG entwickelt oder offiziell endorsed.

---

## 11. Offene Fragen vor Implementation

1. **Struct-Padding verifizieren** — MSVC vs Rust `#[repr(C)]` — sollten identisch sein, aber: einmal mit echtem Capture testen (MSFS läuft, einfaches C++-Tool dumped die ersten 64 Bytes der Struct als Hex, wir vergleichen).

2. **Volatile Header-Updates** — wenn PMDG einen Aircraft-Update rollt, kann der Header sich ändern. Strategie: in jedem Release den `assert size_of` Test mitlaufen lassen + Version-Pin im README.

3. **Throttling** — `PERIOD_ON_SET + FLAG_CHANGED` heißt: nur wenn sich Daten ändern. Aber: bei aktivem Flug ändert sich praktisch alles ständig. Brauchen wir Rate-Limiting clientseitig? Vermutlich nicht — der ClientData-Channel ist effizient designed.

4. **Multiple PMDG aircraft** — falls der Pilot z.B. 737 lädt, dann 777 — müssen wir cleanly unsubscribe von der ersten Definition + neu subscribe zur zweiten. Test-Case.

---

**Ready to start with Phase 5.1.** Implementation in feature-branch, kein main-merge bevor User-Test bestanden.
