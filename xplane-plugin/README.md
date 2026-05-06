# AeroACARS X-Plane Plugin

Native X-Plane plugin (XPLM SDK 4.3.0, C++17) that pairs with the
**AeroACARS** desktop ACARS client (v0.5.0+) to provide frame-perfect
telemetry — most importantly, frame-perfect **touchdown landing-rate
capture**.

| Field          | Value                                                   |
|----------------|---------------------------------------------------------|
| Plugin name    | `AeroACARS Premium`                                     |
| Signature      | `com.aeroacars.xplane.premium`                          |
| Wire format    | Line-delimited JSON over UDP loopback                   |
| Loopback port  | `127.0.0.1:49001`                                       |
| Min X-Plane    | 11.50 (XPLM303)                                         |
| Platforms      | Windows x64 · macOS universal (x86_64+arm64) · Linux x64 |
| License        | Same as the SDK (BSD) — free for commercial use         |

The plugin is **strictly optional**. AeroACARS without it works exactly
as before — the standard X-Plane RREF UDP integration on port 49000
covers every flight. Installing the plugin upgrades the touchdown
detection from "polled at 5 s cadence" to "captured in the exact frame
of wheel contact, with 500 ms lookback for peak descent VS".

## Why a plugin

The RREF UDP protocol is excellent for general telemetry but has two
characteristics that make it unsuitable for landing-rate capture:

1. **Cadence:** the AeroACARS streamer ticks every 5 s, so by the time
   we detect the on-ground edge the buffer of pre-touchdown samples
   has already been evicted.
2. **Smoothing:** `vh_ind_fpm` is the cockpit-display VS, smoothed for
   readability — its value at touchdown is closer to "what the
   pilot's eyes see" than to "what the airframe actually did".

The plugin runs *inside* X-Plane's flight loop. It reads
`fnrml_gear` (gear normal force, in Newtons) every frame, captures
the touchdown edge with frame-perfect timing, and back-references a
500 ms lookback ring buffer to find the peak descent VS — pitch-
corrected to the body axis, just like the established `xgs` plugin.

The result is then fired off as a one-shot UDP "touchdown" packet to
the desktop client, which uses it in preference to its own
RREF-derived edge detection.

## Wire format

Every packet is a single line of JSON terminated with `\n`. The
schema is versioned via `"v":1`. Two packet types:

### `telemetry` — every flight-loop tick

Sent on every flight loop tick (~20 Hz cruise / per-frame near the
ground). Used as a heartbeat — the client uses it to know the plugin
is alive but trusts the standard RREF stream for the live values.

```json
{"v":1,"type":"telemetry","seq":12345,"ts":1234.567890,
 "lat":50.0345678,"lon":8.5712345,
 "agl_ft":2150.40,"vs_fpm_raw":-285.40,"vs_fpm":-285.10,
 "fnrml_gear_n":0.00,"on_ground":false,"g_normal":0.9970,
 "pitch_deg":3.420,"bank_deg":0.150,"hdg_true":253.117,
 "ias_kt":138.40,"gs_kt":134.50}
```

### `touchdown` — one-shot at wheel contact

Fires exactly once per landing, the instant `fnrml_gear` crosses
the touchdown threshold (1 N — far below any physically plausible
contact). The `captured_*` fields hold the values we want to
record for the PIREP: peak descent VS pulled from a 500 ms
lookback, pitch- and bank-attitude at the edge, etc. Re-arms when
AGL climbs back above 50 ft so a touch-and-go gets two events.

```json
{"v":1,"type":"touchdown","seq":12450,"ts":1289.012345,
 "lat":50.0411111,"lon":8.5811111,
 "captured_vs_fpm":-285.4,"captured_g_normal":1.18,
 "captured_pitch_deg":3.4,"captured_bank_deg":0.2,
 "captured_ias_kt":138.0,"captured_gs_kt":134.5,
 "captured_heading_deg":253.1,
 "fnrml_gear_n":52312.0,"agl_ft":0.4}
```

## Installation (pilot-facing)

The AeroACARS desktop installer copies the plugin automatically. If
you're installing manually:

1. Download the `AeroACARS-XPlane-Plugin-vX.Y.Z.zip` artifact from
   the GitHub Release page.
2. Extract it into your X-Plane plugins folder so you end up with:
   ```
   <X-Plane>/Resources/plugins/AeroACARS/
       64/
           win.xpl   (Windows pilots)
           mac.xpl   (macOS pilots)
           lin.xpl   (Linux pilots)
       README.md
       XPLM_SDK_LICENSE.txt
   ```
   You can drop the *whole* folder in — X-Plane picks the matching
   `.xpl` for your platform automatically.
3. Restart X-Plane. Open `Plugins → Plugin Admin` — you should see
   "AeroACARS Premium" listed as enabled.
4. Open the AeroACARS desktop client → Settings → Debug. The
   "X-Plane Premium Plugin" panel should turn green within a few
   seconds.

If the panel doesn't turn green:

* Open `<X-Plane>/Log.txt` and grep for `[AeroACARS]` — every
  log line from the plugin is prefixed with that.
* Check that no other AeroACARS instance is running (port 49001
  is held by exactly one app at a time).

## Building from source

The plugin is in **C++17**, no exceptions, no RTTI, hidden symbol
visibility — same conventions every other open-source XPLM plugin
uses. Cross-platform build via CMake.

### Prerequisites

* CMake ≥ 3.20 (Visual Studio 17 2022 ships one bundled at
  `BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe`)
* Windows: Visual Studio 2022 (Build Tools edition is fine)
* macOS: Xcode 15+ command-line tools
* Linux: GCC 11+ or Clang 14+
* The X-Plane SDK is **vendored** under
  `third_party/XPSDK430/` — no separate download needed.

### Windows

```sh
cd xplane-plugin
cmake -B build -G "Visual Studio 17 2022" -A x64
cmake --build build --config Release --target AeroACARS
# Output: build/AeroACARS/64/win.xpl
```

### macOS (universal — Apple Silicon + Intel in one .xpl)

```sh
cd xplane-plugin
cmake -B build -G "Unix Makefiles" \
      -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64"
cmake --build build --target AeroACARS
# Output: build/AeroACARS/64/mac.xpl
```

### Linux

```sh
cd xplane-plugin
cmake -B build -G "Unix Makefiles" -DCMAKE_BUILD_TYPE=Release
cmake --build build --target AeroACARS
# Output: build/AeroACARS/64/lin.xpl
```

### Output layout

CMake writes files into the X-Plane "fat plugin" layout straight away:

```
build/
  AeroACARS/
    README.md
    XPLM_SDK_LICENSE.txt
    64/
      win.xpl   (or mac.xpl, lin.xpl)
```

Drop the whole `build/AeroACARS/` folder into
`<X-Plane>/Resources/plugins/` to install.

## Architecture & safety

The plugin runs in X-Plane's render thread via the flight-loop
callback. It is constrained by four non-negotiable rules:

1. **Never crash X-Plane.** Every `XPLMFindDataRef` result is
   NULL-checked before use. All errors are caught and logged via
   `XPLMDebugString`, never propagated. C++ exceptions are
   compiled out (`-fno-exceptions`).
2. **Never stall the flight loop.** The callback reads ~15
   DataRefs (microseconds), builds a small JSON string
   (microseconds), and calls a non-blocking `sendto()` on a UDP
   socket (microseconds when the buffer is empty,
   `ECONNREFUSED`-ignored when the client isn't listening).
   No filesystem I/O, no malloc inside the hot path.
3. **Never persist state outside the plugin's address space.**
   No file writes, no registry edits, no env-var tweaks. The
   plugin is purely read-only against X-Plane state.
4. **Clean shutdown on plugin reload.** `XPluginStop` unregisters
   the flight loop, closes the socket, and zeros every DataRef
   handle, so a subsequent `XPluginStart` starts from a known-
   good slate.

See `src/plugin.cpp` for the full implementation — heavily
commented, ~570 lines including blank lines and comments.

## DataRefs read

| DataRef                                                    | Used for                          |
|------------------------------------------------------------|-----------------------------------|
| `sim/flightmodel/position/latitude`                        | telemetry, touchdown lat          |
| `sim/flightmodel/position/longitude`                       | telemetry, touchdown lon          |
| `sim/flightmodel/position/y_agl`                           | adaptive-rate trigger, edge guard |
| `sim/flightmodel/position/local_vy`                        | VS (raw, m/s, no smoothing)       |
| `sim/flightmodel/forces/fnrml_gear`                        | touchdown edge detection          |
| `sim/flightmodel/failures/onground_any`                    | reported in telemetry             |
| `sim/flightmodel2/misc/gforce_normal`                      | g-force capture                   |
| `sim/flightmodel/position/{theta,phi,psi}`                 | pitch / bank / true heading       |
| `sim/cockpit2/gauges/indicators/airspeed_kts_pilot`        | IAS                               |
| `sim/flightmodel/position/groundspeed`                     | GS                                |
| `sim/time/{paused,is_in_replay}`                           | suppress packets while paused     |

All names are read-only. The plugin never writes to a DataRef.

## License & attribution

The plugin source is licensed identically to AeroACARS itself — see
the project root `LICENSE`. The X-Plane SDK headers under
`third_party/XPSDK430/` are licensed under the Laminar Research /
X-Plane Plugin SDK BSD license — see `third_party/XPSDK430/license.txt`.
The license file is copied into the released plugin folder as
`XPLM_SDK_LICENSE.txt` to comply with the attribution clause.
