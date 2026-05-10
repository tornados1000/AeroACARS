# Aircraft-Type-Match — Architektur, Aliases, Maintenance

**Status:** v1.0 — **Approved for implementation** (initial)
**Cutoff:** Forward-only — gilt fuer alle Bids ab AeroACARS v0.7.2
**Vorgaenger:** keine — initial Spec nach Live-Bug 2026-05-10 (MPH62 MD-11)
**Goal:** Pilot wird NIE wieder von einer Cargo-/Variant-Bid blockiert obwohl er das richtige Flugzeug geladen hat. Gleichzeitig: echter Mismatch (z.B. A320 statt B738) wird zuverlaessig erkannt und blockiert.

---

## 0. Warum dieses Dokument

Bei Pilot-Tests sind in 2026 zwei Live-Bugs aufgetreten:

1. **2026-05-04 — Emirates UAE770 (A359-Bid):** Sim hat "A350-900 (No Cabin)" geladen, AeroACARS blockierte mit `aircraft_mismatch` weil ICAO-Code `A359` strict gegen Sim-Title `A350-900` verglichen wurde. → Aliases-Tabelle eingefuehrt.
2. **2026-05-10 — Martinair Cargo MPH62 (MD11-Bid):** Sim hat "TFDi Design MD-11F" geladen, AeroACARS blockierte. → MD-11-Familie hatte keinen Eintrag in der Aliases-Tabelle (klassischer Vergessen-Eintrag, B777F existierte schon).

**Befund:** Aliases-Tabelle wird **ad-hoc gewartet** wenn der naechste Live-Bug auftritt. Das ist nicht systematisch — wir brauchen:
- Klare Architektur was matched warum
- Liste aller bekannten ICAO-Familien die Piloten realistisch fliegen
- Test-Matrix die jede Familie pro Bug-Anfaelligkeit deckt
- Maintenance-Workflow wenn ein neuer Live-Bug kommt

Diese Spec ist die Antwort. Kein neues Verhalten — sondern Dokumentation des Status-Quo + Lueckenanalyse + Prozess.

---

## 1. Scope

### Was DRIN ist

| # | Inhalt |
|---|---|
| §2 | Aktuelle Architektur (`aircraft_aliases` + `aircraft_types_match` + `title_mentions_icao`) |
| §3 | Inventur der vorhandenen Aliases (49 Eintraege per v0.7.2) |
| §4 | Lueckenanalyse: bekannte fehlende Aircraft-Familien |
| §5 | Test-Matrix pro Aircraft-Familie + Bug-Klasse |
| §6 | Maintenance-Workflow (was tun wenn ein Live-Bug kommt) |
| §7 | Datenkontrakt-Garantien (was matched, was nicht) |
| §8 | Backward-Compat + Migrations-Strategie |

### Was BEWUSST NICHT in dieser Spec

- **Algorithmus-Refactor.** Die Substring-Match-Logik (`alias.iter().any(|a| title.contains(a))`) bleibt. Sie ist pragmatisch und funktioniert seit 2026-05-04.
- **Strict-Mode pro VA.** Manche VAs wollen evtl. striktere Matches (z.B. nur exakte Frachter-Variante). Das ist v0.8.x-Diskussion, nicht hier.
- **Type-Database-Sync mit phpVMS.** Wir spiegeln die phpVMS-`aircrafts.icao`-Werte nicht nach AeroACARS — wir matchen tolerant gegen den vom Sim gemeldeten Title.

---

## 2. Architektur

### 2.1 Komponenten

```
                  ┌─────────────────────────┐
phpVMS Bid ──────▶│   expected_icao         │
("MD11")          │   (4-letter ICAO code)  │
                  └────────────┬────────────┘
                               │
                               ▼
                  ┌─────────────────────────┐
Sim Snapshot ────▶│ aircraft_types_match()  │ ──▶  bool
("MD11F" + Title  │  + title_mentions_icao  │
 "TFDi Design     │                         │
  MD-11F PW...")  └────────────┬────────────┘
                               │
                               ▼
                  ┌─────────────────────────┐
                  │ aircraft_aliases()      │  ◀──  pflegbare
                  │   "MD11" → ["MD-11",    │       Tabelle
                  │             "MD11"]     │
                  └─────────────────────────┘
```

### 2.2 Pruef-Reihenfolge

```rust
// lib.rs:5215
let types_match_loose   = aircraft_types_match(expected, actual);
let title_supports      = title_mentions_icao(&sim_title, expected);
if !types_match_loose && !title_supports {
    return Err(UiError::new("aircraft_mismatch", ...));
}
```

**Alternative-Pruefungen, BEIDE muessen fehlschlagen** damit blockiert wird:

| Pruefung | Pfad |
|---|---|
| `aircraft_types_match(expected, actual)` | ICAO ↔ ICAO mit Aliases (z.B. `MD11` ↔ `MD11F`) |
| `title_mentions_icao(sim_title, expected)` | Title-Substring-Suche (z.B. "TFDi Design **MD-11F**...") |

Das macht den Match **tolerant** — sobald EINE der beiden Pruefungen passt, geht der Flug los. Strict-Mismatch (z.B. A320 statt B738) wird nur dann blockiert wenn **beide** Pruefungen fehlschlagen.

### 2.3 `aircraft_aliases` Format

```rust
fn aircraft_aliases(code: &str) -> &'static [&'static str] {
    match code {
        "MD11"  => &["MD-11", "MD11"],
        "MD11F" => &["MD-11F", "MD11F"],
        ...
        _ => &[],  // Fallback: strict equality only
    }
}
```

**Aliases sind Substring-Patterns die im Sim-Title gesucht werden.** Es ist **kein bidirektionales Mapping** ICAO↔ICAO sondern eine Liste von Strings die "wahrscheinlich im Title vorkommen wenn das richtige Flugzeug geladen ist".

### 2.4 Symmetrie

`aircraft_types_match` prueft **beide Richtungen**:

```rust
aircraft_aliases(&exp).iter().any(|alias| act.contains(alias))
    || aircraft_aliases(&act).iter().any(|alias| exp.contains(alias))
```

Damit funktioniert sowohl:
- Bid `MD11` + Sim-ICAO `MD11F` → `aliases("MD11")=["MD-11", "MD11"]`, `"MD11F".contains("MD11")` ✓
- Bid `MD11F` + Sim-ICAO `MD-11F` → `aliases("MD11F")=["MD-11F", "MD11F"]`, `"MD-11F".contains("MD-11F")` ✓

---

## 3. Inventur (Stand v0.7.2)

49 ICAO-Codes mit expliziten Aliases. Pro Familie:

### 3.1 Airbus (16)

| Familie | ICAO-Codes mit Alias |
|---|---|
| A220 | `BCS1`, `BCS3` |
| A320 ceo+neo | `A318`, `A319`, `A320`, `A321`, `A20N`, `A21N` |
| A330 | `A332`, `A333`, `A338`, `A339` |
| A340 | `A342`, `A343`, `A345`, `A346` |
| A350 | `A359`, `A35K` |
| A380 | `A388` |

### 3.2 Boeing (22)

| Familie | ICAO-Codes mit Alias |
|---|---|
| 717 | `B712` |
| 737 NG | `B736`, `B737`, `B738`, `B739` |
| 737 MAX | `B37M`, `B38M`, `B39M`, `B3XM` |
| 747 | `B741`, `B742`, `B744`, `B748` |
| 757 | `B752`, `B753` |
| 767 | `B762`, `B763`, `B764` |
| 777 | `B772`, `B77L`, `B773`, `B77W`, `B77F` |
| 787 | `B788`, `B789`, `B78X` |

### 3.3 McDonnell Douglas (2 — NEU v0.7.2)

| Familie | ICAO-Codes mit Alias |
|---|---|
| MD-11 | `MD11`, `MD11F` |

### 3.4 Embraer (6)

| Familie | ICAO-Codes mit Alias |
|---|---|
| ERJ | `E170`, `E175`, `E190`, `E195` |
| E2 | `E290`, `E295` |

### 3.5 Test-Coverage (10 Tests in `aircraft_alias_tests`)

| Test | Was es deckt |
|---|---|
| `a359_matches_a350_900_long_form` | Live-Bug 2026-05-04 (Emirates A359) |
| `b738_matches_737_800` | 737-NG-Pfad |
| `b77w_matches_777_300er` | 777-300ER mit + ohne Leerzeichen |
| `a20n_matches_a320neo` | NEO-Variation NEO/-NEO/" NEO" |
| `b789_matches_787_9` | 787-Pfad |
| `unrelated_types_dont_match` | False-Positive-Schutz B738 ≠ A320 |
| `case_insensitive` | Lowercase-Inputs |
| `strict_equality_still_works_for_unaliased` | DH8D fallback |
| **`md11_matches_md_11f_long_form`** | **Live-Bug 2026-05-10 (MD-11)** |
| **`md11_does_not_match_unrelated_widebodies`** | MD11 ≠ B77W/A359/B748 |

---

## 4. Lueckenanalyse

Diese ICAO-Codes haben **keinen Alias** in der Tabelle und wuerden bei einer GSG-Bid blockieren wenn der Sim-Title nicht den ICAO-Code als Substring enthaelt:

### 4.1 Cargo-Varianten (HOHE Prio — Cargo-Operatoren wie Martinair, Lufthansa Cargo, FedEx etc.)

| ICAO-Code | Real-Name | Sim-Adapter (typisch) | Status |
|---|---|---|---|
| `B74F` | 747-Frachter (Klassiker) | div. | **fehlt** |
| `B748F` | 747-8F | PMDG, Salty | **fehlt** |
| `B74S` | 747SP / -200F | div. | **fehlt** |
| `B752F` | 757-200F | DHL/UPS | **fehlt** |
| `B763F` | 767-300F | FedEx, ABX | **fehlt** |
| `B764F` | 767-400F (selten, kein realer Frachter) | — | n/a |
| `B788F` | 787-Frachter (existiert nicht real) | — | n/a |
| `A332F` | A330-200F | Quatar, Turkish | **fehlt** |
| `A338F` | A330-800F | (selten) | **fehlt** |
| `A33F` / `A330F` | Generischer A330F | div. | **fehlt** |

### 4.2 Regional / Turboprop (MITTLERE Prio — kleinere VAs, Trainings-Bids)

| ICAO-Code | Real-Name | Sim-Adapter | Status |
|---|---|---|---|
| `AT72` | ATR 72 | Asobo, FlightSimGroup | **fehlt** |
| `AT76` | ATR 72-600 | gleicher Adapter | **fehlt** |
| `AT43` | ATR 42 | seltener | **fehlt** |
| `DH8D` | Dash 8 Q400 | Aerosoft, Asobo | (strict OK weil Sim oft "DHC8-400" + Pilot bid auch DH8D) |
| `DH8C` | Dash 8 Q300 | seltener | **fehlt** |
| `CRJ7` | CRJ-700 | Aerosoft | **fehlt** |
| `CRJ9` | CRJ-900 | Aerosoft | **fehlt** |
| `CRJX` | CRJ-1000 | (selten) | **fehlt** |

### 4.3 Andere Hersteller (NIEDRIGE Prio — Spezialfaelle)

| ICAO-Code | Real-Name | Sim-Adapter | Status |
|---|---|---|---|
| `MD80` / `MD82` / `MD83` / `MD88` | MD-80-Familie | LeonardoSH | **fehlt** |
| `MD90` | MD-90 | (sehr selten) | **fehlt** |
| `F70` / `F100` | Fokker 70/100 | Just Flight | **fehlt** |
| `BCS3` ist da | A220-300 | — | OK |
| `SU95` | Sukhoi Superjet 100 | SimCol | **fehlt** |
| `C25A` / `C56X` | Citation CJ2/Excel | Asobo | (strict OK weil Sim meistens "CJ2" oder "Citation" — pruefen) |

### 4.4 Generic-Aliases die wir absichtlich nicht haben

- **`A330` ohne Variant** — wird nicht gepflegt, weil phpVMS-Standard nur die spezifischen `A332/A333/A338/A339` kennt. Falls eine VA generisch `A330` als Bid-ICAO benutzt → das ist VA-Datenfehler, nicht AeroACARS-Bug.

---

## 5. Test-Matrix (verbindlich)

Pro **Aircraft-Familie** muessen die folgenden Tests existieren:

| Test-Klasse | Was es testet | Pflicht? |
|---|---|---|
| **Long-Form-Match** | `aircraft_types_match(ICAO, "Long Form")` ist `true` (z.B. `A359` ↔ `A350-900`) | Ja, pro Familie |
| **Cargo-Variant-Match** | Pax-ICAO matched Frachter-Title (z.B. `MD11` ↔ `MD-11F`) | Ja, wenn Cargo-Variant existiert |
| **Strict-Cargo-Match** | Reine Frachter-ICAO matched nur Frachter-Title (z.B. `MD11F` ↔ `MD-11F`) | Ja, wenn Cargo-Variant existiert |
| **Unrelated-Mismatch** | Verwandte aber unterschiedliche Familien matchen NICHT (z.B. `MD11` ≠ `B748`) | Ja, pro Familie |
| **Case-Insensitive** | Lowercase-Inputs funktionieren (`md11` ↔ `md-11f`) | Ja, mindestens ein Test pro Familie |
| **Variant-Suffix-Variations** | NEO/MAX/ER mit/ohne Bindestrich/Leerzeichen (`A20N` ↔ `A320NEO`/`A320-NEO`/`A320 NEO`) | Wenn Familie Variant-Suffixe hat |

### 5.1 Verifikations-Befehl

```sh
cargo test --lib aircraft_alias_tests
```

Sollte **>= N Tests** liefern, wo N = (Anzahl Familien aus §3) × (Anzahl Pflicht-Klassen aus §5).

Aktuell (v0.7.2): 10 Tests fuer 14 Familien (nicht jede Familie hat alle Klassen — siehe §5.2).

### 5.2 Test-Coverage-Soll (v0.7.3 Ziel)

Mindestens ein Long-Form-Match-Test pro Familie + ein Unrelated-Mismatch-Test pro Familien-Paar das verwechselt werden koennte. Aktuell fehlen:

- A330-Familie Long-Form-Match
- A340 Long-Form-Match
- 747-Familie Long-Form-Match
- 757 Long-Form-Match
- 767 Long-Form-Match
- E170/E175/E190/E195 Long-Form-Match
- Embraer-E2 Long-Form-Match
- BCS1/BCS3 Long-Form-Match

---

## 6. Maintenance-Workflow

Wenn ein Pilot mit `aircraft_mismatch` blockiert wird obwohl er das richtige Flugzeug hat:

### 6.1 Schritt-fuer-Schritt

1. **Banner-Text aufschreiben** — enthaelt `bid wants <ICAO> (<reg>), sim has <ICAO_actual> (title "<title>")`
2. **Verifizieren** dass es ein false-positive ist (Pax-ICAO vs Frachter im Sim, Long-Form-Title etc.)
3. **Familie identifizieren** — gehoert das Flugzeug ins selbe Airframe wie die Bid?
4. **Aliases-Tabelle erweitern** in `client/src-tauri/src/lib.rs:408-487`:
   ```rust
   "<ICAO_BID>"    => &["<Long-Form-Substring>", "<ICAO_BID>"],
   "<ICAO_ACTUAL>" => &["<Long-Form-Substring>", "<ICAO_ACTUAL>"],  // bei Cargo separat
   ```
5. **Tests hinzufuegen** in `aircraft_alias_tests`:
   - Long-Form-Match (`<ICAO_BID>` ↔ `<Long-Form>`)
   - Cargo-Variant-Match (falls applicable)
   - Unrelated-Mismatch (gegen mindestens 2 unverwandte Familien)
6. **Spec updaten** — Inventur §3 erweitern, Lueckenanalyse §4 reduzieren
7. **Hotfix-Release** (Patch-Version, z.B. v0.7.X+1)

### 6.2 Beispiel-Diff (v0.7.2 MD-11 Hotfix)

```diff
+ // ---- McDonnell Douglas ----
+ "MD11"  => &["MD-11", "MD11"],
+ "MD11F" => &["MD-11F", "MD11F"],

  // ---- Embraer ----

+ #[test]
+ fn md11_matches_md_11f_long_form() {
+     assert!(aircraft_types_match("MD11", "MD-11F"));
+     assert!(aircraft_types_match("MD11", "MD11F"));
+     ...
+ }
```

5 Zeilen Tabelle + 2 Tests + Spec-Update + CHANGELOG-Eintrag. Ende-zu-Ende ~30 Min inkl. Tag/Build/Release.

### 6.3 Anti-Patterns (NICHT machen)

- **Generic-Wildcard-Match wie `"MD11" => &["MD"]`** — wuerde A220 (CSeries hat keinen "MD" aber andere Hersteller schon) oder weitere McDonnell Douglas-Modelle (MD-80, MD-90) mit-matchen. Aliases muessen **spezifisch genug** sein dass nur die echte Familie matched.
- **`acknowledge_aircraft_mismatch` Bypass dauerhaft setzen** — der Modal-Bypass im VFR-Pfad ist absichtlich Pro-Flug, nicht Pro-Pilot. Wer das Aliase-Problem mit "Trotzdem starten" loest, lernt dass die Alias-Tabelle nicht gepflegt wird.
- **Sim-Title-Spezifika hardcoden** — `"TFDi Design MD-11F"` als Alias ist falsch, weil PMDG/Captain-Sim/etc. andere Title-Strings produzieren. Aliase sind die **gemeinsame Familien-Bezeichnung** ("MD-11F"), nicht der Vendor-Praefix.

---

## 7. Daten-Vertrag

### 7.1 Was matched (Garantie)

| Bid-ICAO | Sim-Title (typisch) | Match? |
|---|---|---|
| `A359` | "Airbus A350-900 [Asobo]" | ✓ via Long-Form-Alias |
| `A359` | "A350-900 (No Cabin)" | ✓ via Long-Form-Alias |
| `B738` | "PMDG 737-800" | ✓ via Long-Form-Alias |
| `B77W` | "PMDG 777-300ER" | ✓ via Long-Form-Alias |
| `A20N` | "A320NEO Asobo" | ✓ via NEO-Variant-Alias |
| `MD11` | "TFDi Design MD-11F PW4462 (Low Poly Cabin)" | ✓ via §3.3 (v0.7.2+) |
| `MD11F` | "TFDi Design MD-11F PW4462 (Low Poly Cabin)" | ✓ via §3.3 (v0.7.2+) |
| `B77F` | "PMDG 777-200F" | ✓ via §3.2 |
| `<ICAO>` | "<Long Form including ICAO>" | ✓ via title_mentions_icao Fallback |

### 7.2 Was NICHT matched (Garantie)

| Bid-ICAO | Sim-Title | Match? |
|---|---|---|
| `B738` | "Airbus A320 NEO" | ✗ unrelated families |
| `MD11` | "Boeing 747-8" | ✗ unrelated widebodies |
| `A359` | "Boeing 777-300ER" | ✗ unrelated widebodies |
| `B788` | "Asobo 787-9" | ✗ verschiedene Variant (B788 ≠ B789) |

### 7.3 Was unklar/edge-case ist

| Bid-ICAO | Sim-Title | Match? |
|---|---|---|
| `MD11F` | "Just Flight MD-11" (Pax-Title) | **✗** — strict gewollt: Cargo-Bid darf nicht in Pax-Sim fliegen (Cargo-Compartment fehlt) |
| `MD11` | "DC-10-30" | ✗ — DC-10 ist Vorlauefer, nicht MD-11 |
| `B748` | "Boeing 747-8 Freighter (PMDG)" | **fragwuerdig** — B748 ist normalerweise Pax-Variante, B748F waere Frachter. Akt. matched via Long-Form-Substring "747-8". TODO: separater B748F-Alias falls Cargo-Bids reinkommen |

---

## 8. Backward-Compat + Migrations-Strategie

- **Aliases sind additiv.** Eintraege werden nur hinzugefuegt, nie entfernt — sonst wuerden bestehende Pilot-PIREPs ploetzlich blockiert werden.
- **Test-Coverage retro-aktiv.** Wenn ein neuer Test einen unrelated-mismatch zeigt, der vorher als false-positive durchging, dokumentieren wir das in §7.3 als "edge case" statt zu blocken.
- **VA-spezifische Aliases**: nicht in dieser Tabelle. Wenn z.B. eine VA "MD11" bewusst nur als reine Pax-Variante zaehlen will, ist das ein **VA-Daten-Job** (in phpVMS `aircraft.icao = MD11F` setzen wenn Cargo). AeroACARS macht keine VA-spezifische Logik.

### 8.1 Wann Aliases REMOVEN okay waere

- Wenn ein bestehender Alias **nachweislich** false-positives produziert (z.B. weil zwei Hersteller den gleichen Long-Form-String benutzen). Dann: Alias durch praeziseren ersetzen + Test der den Edge-Case fixt.
- Beispiel: angenommen ein hypothetischer Sukhoi-SU95-Title "Superjet-100" wuerde von einem A350-Alias "100" gematched werden. Dann waere "100" als alias zu generisch — Pruefung: `aircraft_aliases("A35K") = ["A350-1000"]` ist spezifisch genug, kein Konflikt.

---

## 9. Anhang A — Code-Anker

| Datei | Zeile | Inhalt |
|---|---|---|
| `client/src-tauri/src/lib.rs` | 380-390 | `title_mentions_icao` |
| `client/src-tauri/src/lib.rs` | 408-497 | `aircraft_aliases` Tabelle |
| `client/src-tauri/src/lib.rs` | 494-508 | `aircraft_types_match` |
| `client/src-tauri/src/lib.rs` | 5209-5234 | Match-Aufruf + Block-Logik in `flight_start` (IFR) |
| `client/src-tauri/src/lib.rs` | 5953-5970 | Match-Aufruf + Warning-Logik in `flight_start_manual` (VFR) |
| `client/src-tauri/src/lib.rs` | 17078-17158 | `aircraft_alias_tests` Modul |

---

## 10. Glossar

- **ICAO-Code (4-letter):** Eindeutiger Aircraft-Type-Code laut ICAO Doc 8643. Beispiele: `A359`, `B738`, `MD11`, `MD11F`. **Source-of-Truth fuer phpVMS-Bid-Definitionen.**
- **Sim-Title:** Volltext-String den der Simulator in der `TITLE`-SimVar liefert. Beispiele: "Airbus A350-900 [Asobo]", "TFDi Design MD-11F PW4462". Vendor-spezifisch.
- **Long-Form-Alias:** Substring-Pattern das im Sim-Title vorkommt wenn das richtige Flugzeug geladen ist. Beispiel: `"A350-900"` ist Alias fuer `A359`.
- **Cargo-Variant:** F-Suffix (z.B. `MD11F`, `B77F`). Eigene ICAO weil Cargo-Compartment + andere MTOW.
- **Pax-Variant:** Standard-ICAO ohne F-Suffix. In dieser Spec wird bewusst toleriert dass Pax-Bid einen Cargo-Sim akzeptiert (gleiche Familie, Cargo-Pilot kann Pax fliegen). Umgekehrt **NICHT** (Cargo-Bid darf nicht in Pax-Sim).

---

**Ende der Spec v1.0 — Maintenance-Prozess in §6 ist verbindlich. Naechster Update: §4 Lueckenanalyse abarbeiten, geplant fuer v0.7.3.**
