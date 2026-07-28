# Discord-RPC Asset Pack (v0.9.0)

5 PNG assets for the pilot client's Discord Rich Presence.
To be uploaded once by the VA owner via the Discord Developer Portal.

## What's in here

| File | Asset key | Use in the Discord profile |
|---|---|---|
| `aeroacars_logo.png` | `aeroacars_logo` | `large_image` — large image next to the pilot status |
| `sim_msfs2024.png`   | `sim_msfs2024`   | `small_image` — small badge bottom-right of the logo, MSFS 2024 aircraft |
| `sim_msfs2020.png`   | `sim_msfs2020`   | same, MSFS 2020 aircraft |
| `sim_xplane12.png`   | `sim_xplane12`   | same, X-Plane 12 aircraft |
| `sim_xplane11.png`   | `sim_xplane11`   | same, X-Plane 11 aircraft |

All 1024×1024 PNG, rounded corners, aviation theme.

## Upload instructions (one-time)

1. Open <https://discord.com/developers/applications>
2. Select the AeroACARS app
3. Sidebar → **Rich Presence** → **Art Assets**
4. Click **Add Image(s)**
5. Upload all 5 PNGs
6. Discord automatically takes the file name (without `.png`) as the asset key —
   **do NOT rename**, otherwise the pilot client won't be able to find it
7. **Save Changes**

Discord caches new assets for about 10 minutes. If the images don't show up
immediately during testing, wait a bit or restart the Discord client.

## How the code references them

Asset keys are hardcoded in:

- [`client/src-tauri/crates/discord-presence/src/format.rs`](../../client/src-tauri/crates/discord-presence/src/format.rs)
  functions `sim_to_asset_key()` and constant `ASSET_LOGO`

If you want to rename asset keys, you MUST update both the files here and
the constants in the Rust code.

## Regenerating

If you change the logo (e.g. a new 1024×1024 source asset) or want to tune
the colors/layout of the sim badges:

```bash
python docs/discord-assets/generate.py
```

The generator uses the existing Tauri icon
(`client/src-tauri/icons/icon.png`) as the logo source and generates the
sim badges programmatically (PIL/Pillow required).

## Spec

Detailed layout decisions: [`docs/spec/v0.9.0-discord-rich-presence.md`](../spec/v0.9.0-discord-rich-presence.md)
(section LE4 Asset Layout).
