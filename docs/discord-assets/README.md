# Discord-RPC Asset-Pack (v0.9.0)

5 PNG-Assets fuer die Discord Rich Presence des Pilot-Clients.
Hochzuladen einmalig beim VA-Owner via Discord-Developer-Portal.

## Was hier liegt

| Datei | Asset-Key | Verwendung im Discord-Profil |
|---|---|---|
| `aeroacars_logo.png` | `aeroacars_logo` | `large_image` — grosses Bild links neben dem Pilot-Status |
| `sim_msfs2024.png`   | `sim_msfs2024`   | `small_image` — kleines Badge unten-rechts am Logo, MSFS-2024-Flieger |
| `sim_msfs2020.png`   | `sim_msfs2020`   | dito, MSFS-2020-Flieger |
| `sim_xplane12.png`   | `sim_xplane12`   | dito, X-Plane-12-Flieger |
| `sim_xplane11.png`   | `sim_xplane11`   | dito, X-Plane-11-Flieger |

Alle 1024×1024 PNG, abgerundete Ecken, Aviation-Theme.

## Upload-Anleitung (einmalig)

1. <https://discord.com/developers/applications> oeffnen
2. AeroACARS-App auswaehlen
3. Sidebar → **Rich Presence** → **Art Assets**
4. **Add Image(s)** klicken
5. Alle 5 PNGs hochladen
6. Discord uebernimmt automatisch den Dateinamen (ohne `.png`) als Asset-Key —
   **NICHT umbenennen**, sonst findet sie der Pilot-Client nicht
7. **Save Changes**

Discord cached neue Assets ca. 10 min. Falls die Bilder beim Test nicht
sofort kommen, kurz warten oder Discord-Client neu starten.

## Wie der Code sie referenziert

Asset-Keys sind hartcodiert in:

- [`client/src-tauri/crates/discord-presence/src/format.rs`](../../client/src-tauri/crates/discord-presence/src/format.rs)
  Funktionen `sim_to_asset_key()` und Konstante `ASSET_LOGO`

Wenn du Asset-Keys umbenennen willst, MUSST du sowohl die Dateien hier als
auch die Konstanten im Rust-Code anpassen.

## Neu generieren

Wenn du das Logo aenderst (z.B. neues 1024×1024 Source-Asset) oder die
Farben/Layout der Sim-Badges tunen willst:

```bash
python docs/discord-assets/generate.py
```

Der Generator nutzt das vorhandene Tauri-Icon
(`client/src-tauri/icons/icon.png`) als Logo-Quelle und generiert die
Sim-Badges programmatisch (PIL/Pillow erforderlich).

## Spec

Detail-Layout-Entscheidungen: [`docs/spec/v0.9.0-discord-rich-presence.md`](../spec/v0.9.0-discord-rich-presence.md)
(Sektion LE4 Asset-Layout).
