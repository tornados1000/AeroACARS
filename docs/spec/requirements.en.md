# Requirements Specification — CloudeAcars

**Version:** 1.0 (Kickoff)
**Date:** 2026-05-01
**Language:** German (client's original document)

> This document is the unabridged original specification as handed over by the client at project kickoff. It is the normative source for phases 1–5. Changes are tracked via ADRs ([`../decisions/`](../decisions/)) and protocol/architecture updates, **not** by editing this file.

---

## 1. Purpose of the Software

A modern ACARS client software is to be developed that communicates with **phpVMS 7**, automatically records flights, and transmits live information to the website.

The software should function similarly to **vmsACARS** or **smartCARS**, but with a particular focus on:

* native support for **Windows and macOS**
* support for **MSFS 2020, MSFS 2024, X-Plane 11, and X-Plane 12**
* direct communication with **phpVMS 7**
* live tracking on the website
* complete flight recording
* automatic PIREP submission
* detailed takeoff, runway, and landing analysis
* storage of flight log, position data, METAR, landing rate, G-force, runway deviations, and other quality data

---

## 2. Supported Operating Systems

### Must-Have Requirement

The ACARS software must be available as native desktop software for the following systems:

| Operating System | Requirement                    |
| ----------------- | ------------------------------ |
| Windows           | Must be supported               |
| macOS              | Must be natively supported       |
| Linux              | Optional / future extension |

The macOS version must not work only via a Windows VM or Wine. Genuine native macOS support is expected.

---

## 3. Supported Simulators

The software must support at least the following simulators:

| Simulator                   | Version | Requirement |
| --------------------------- | ------: | ----------- |
| Microsoft Flight Simulator  |    2020 | Must        |
| Microsoft Flight Simulator  |    2024 | Must        |
| X-Plane                     |      11 | Must        |
| X-Plane                     |      12 | Must        |

Optionally, the architecture should later be extensible to further simulators such as Prepar3D or FSX.

---

## 4. Basic Functionality

The software should allow the pilot to load a flight from phpVMS, start it in the simulator, record the flight live, and send a PIREP to phpVMS automatically or manually after completion.

### Must-Have Functions

* Login with phpVMS URL and API key
* Retrieval of pilot data
* Retrieval of bids
* Retrieval of flight plans
* Retrieval of fleet and aircraft data
* Selection of a flight
* Selection or verification of the aircraft
* Connection to the simulator
* Start of flight recording
* Live transmission to phpVMS
* Creation of a flight log
* Recording of position data
* Detection of flight phases
* Submission of a complete PIREP
* Storage of custom fields
* Error and offline handling

---

## 5. phpVMS 7 Integration

The software must connect to phpVMS 7 via clean API communication.

### Must-Have Requirements

* Communication exclusively over HTTPS
* Authentication via phpVMS API key
* Use of JSON for API requests and responses
* Correct handling of API errors
* Support for rate limits
* No storage of credentials in plain text
* Secure storage of the API key in the operating system
* Compatibility with phpVMS 7
* Compatibility with phpVMS table prefix
* No changes to the phpVMS core, wherever possible
* Provision of a dedicated phpVMS module or API connector

---

## 6. Data Retrieval from phpVMS

The ACARS software must be able to retrieve the following data from phpVMS:

### Pilot Data

* Name
* Pilot ID
* Airline
* Home airport
* Current location
* Rank
* Status
* Permissions
* Existing bids

### Flight Plan Data

* Airline
* Flight number
* Route code
* Route leg
* Departure airport
* Arrival airport
* Alternate airport
* Planned route
* Planned flight time
* Distance
* Flight type
* Cruise altitude
* SimBrief information, if available

### Aircraft and Fleet Data

* Aircraft ID
* Registration / tail number
* ICAO type
* Airline
* Subfleet
* Current location
* Status
* Allowed / not allowed for the flight
* Maintenance status, if available
* Blocked / in-use status, if available

---

## 7. Flight Preparation in the Client

Before departure, the pilot should be able to review and prepare all important information.

### Must-Have Functions

* Select a bid
* Load a flight from the phpVMS flight plan
* Optionally create a charter flight, if permitted
* Select an aircraft
* Detect aircraft mismatch
* Check departure / arrival / alternate
* Display route
* Import or retrieve SimBrief OFP
* Display or set cruise altitude
* Record block fuel
* Record payload / ZFW
* Display planned flight time
* Display planned distance

### Should-Have Functions

* Import of flight plan files
* Display of route on a map
* Display of weather / METAR
* Warning for incorrect departure airport
* Warning for incorrect aircraft
* Warning for missing SimBrief plan

---

## 8. Simulator Data and Telemetry

The software must continuously capture data from the simulator during the flight.

### Minimum Data

* Latitude
* Longitude
* Altitude MSL
* Altitude AGL
* Heading
* Groundspeed
* IAS
* TAS
* Vertical speed
* Pitch
* Bank
* On-ground status
* Parking brake
* Gear status
* Flaps position
* Engine status
* Fuel quantity
* Fuel used
* Payload
* ZFW
* G-force
* Touchdown G-force
* Landing rate
* Stall warning
* Overspeed warning
* Pause status
* Slew mode
* Simulation rate
* Squawk
* COM/NAV frequencies
* Wind direction
* Wind speed
* QNH
* Aircraft type
* Simulator version

---

## 9. Flight Phases

The software must automatically detect and log flight phases.

### Minimum Phases

* Preflight
* Boarding
* Pushback
* Taxi out
* Takeoff roll
* Takeoff
* Climb
* Cruise
* Descent
* Approach
* Final
* Landing
* Taxi in
* Blocks on
* Arrived
* PIREP submitted

### Example Flight Log

```text
ACARS connected.
Simulator detected: Microsoft Flight Simulator 2024.
Aircraft detected: Airbus A350-900.
Flight DLH123 loaded from phpVMS.
Boarding started.
Pushback started.
Taxi out.
Takeoff from runway 25C.
Climbing through 10,000 ft.
Cruise altitude reached.
Top of descent.
Approach started.
Landing detected on runway 07R.
Blocks on.
PIREP submitted.
```

---

## 10. Live Tracking to the Website

During the flight, the software must regularly send live data to phpVMS.

### Must-Have Data for Live Map

* Pilot ID
* Pilot name
* Callsign
* Flight number
* Aircraft registration
* Aircraft ICAO
* Departure airport
* Arrival airport
* Latitude
* Longitude
* Altitude
* Groundspeed
* Heading
* Vertical speed
* Flight phase
* Distance flown
* Distance remaining
* Estimated time enroute
* Estimated time remaining
* Route progress
* Last update timestamp
* Online network, if detected

### Requirements

* Update at configurable intervals
* No overloading of the phpVMS API
* Automatic reconnection on connection loss
* Storage of unsent position data in a local queue
* Display of the flight on the phpVMS live map

---

## 11. Flight Log / Event Recording

The software must produce a complete flight log.

### Must-Have Events

* ACARS started
* Connection to phpVMS established
* Simulator detected
* Flight loaded
* Aircraft detected
* Aircraft mismatch detected
* Flight started
* Boarding
* Pushback
* Taxi
* Takeoff
* Airborne
* Gear up / down
* Flaps changes
* Passing 10,000 ft
* Cruise reached
* Top of descent
* Approach
* Final
* Touchdown
* Landing rate
* Landing G-force
* Bounce detection
* Taxi in
* Blocks on
* Flight ended
* PIREP submitted

### Should-Have

* Rule-based events
* Configurable thresholds
* Debounce / timeout against log spam
* Admin configuration for events

---

## 12. PIREP Submission

After flight completion, a complete PIREP must be transmitted to phpVMS.

### Must-Have Fields

* Airline
* Flight number
* Route code
* Route leg
* Departure airport
* Arrival airport
* Alternate airport
* Aircraft ID
* Aircraft registration
* Aircraft ICAO
* Flight time
* Block time
* Taxi out time
* Taxi in time
* Planned distance
* Actual distance
* Planned route
* Actual route
* Cruise altitude
* ZFW
* Payload
* Block fuel
* Fuel used
* Remaining fuel
* Landing rate
* Landing G-force
* Flight log
* ACARS positions
* Source name
* Simulator
* Client version
* Custom fields
* Raw data / debug data optional

---

## 13. Runway, Takeoff, and Landing Analysis

The software should perform a detailed analysis of takeoff and landing.

This requires a suitable runway database. It must contain at least runway identifier, coordinates, heading, length, width, and threshold positions.

### Required Runway Data

* Airport ICAO
* Runway ident
* Runway true heading
* Runway magnetic heading
* Runway length
* Runway width
* Runway start threshold latitude / longitude
* Runway end latitude / longitude
* Displaced threshold, if present
* Elevation
* Surface type

---

## 14. Departure Runway Detection

The software must automatically detect which runway was used for departure.

### Detected Based On

* Position during takeoff roll
* Position at liftoff
* Aircraft heading at departure
* Nearest runway centerline
* Distance to the runway
* Departure airport from the flight plan
* Runway heading tolerance

### Fields to Be Stored

```text
departure_runway_ident
departure_runway_heading
departure_runway_detected_lat
departure_runway_detected_lon
departure_runway_confidence
departure_runway_heading_deviation
departure_metar_raw
departure_metar_time
```

---

## 15. Arrival Runway Detection

The software must automatically detect which runway was used for landing.

### Detected Based On

* Touchdown position
* Aircraft heading at touchdown
* Nearest runway centerline
* Distance to the threshold
* Distance to the runway centerline
* Arrival airport from the flight plan
* Groundspeed
* On-ground transition
* Runway heading tolerance

### Fields to Be Stored

```text
arrival_runway_ident
arrival_runway_heading
touchdown_lat
touchdown_lon
touchdown_heading
touchdown_groundspeed
touchdown_vertical_speed
arrival_runway_confidence
arrival_metar_raw
arrival_metar_time
```

---

## 16. Arrival Centerline Deviation

The software must calculate how far laterally the aircraft was from the runway centerline at touchdown.

### Unit

* Meters
* Optionally feet

### Proposed Rating

| Deviation  | Rating         |
| ---------: | -------------- |
|      0–5 m | Very good      |
|     5–10 m | Good           |
|    10–20 m | Acceptable     |
|    20–35 m | Notice         |
|      >35 m | Rule violation |

### Fields to Be Stored

```text
arrival_centerline_deviation_m
arrival_centerline_deviation_ft
arrival_centerline_score
```

---

## 17. Arrival Heading Deviation

The software must calculate how much the aircraft's heading deviates from the runway heading at touchdown.

### Proposed Rating

| Deviation  | Rating         |
| ---------: | -------------- |
|       0–3° | Very good      |
|       3–6° | Good           |
|      6–10° | Acceptable     |
|     10–15° | Notice         |
|       >15° | Rule violation |

### Fields to Be Stored

```text
arrival_heading_deviation_deg
arrival_heading_score
```

---

## 18. Landing G-Force

The software must store the G-force at touchdown.

### Proposed Rating

|     G-Force | Rating                        |
| ----------: | ----------------------------- |
|     <1.30 G | Very soft                     |
| 1.30–1.60 G | Normal                        |
| 1.60–1.90 G | Hard                          |
| 1.90–2.20 G | Very hard                     |
|     >2.20 G | Hard landing / rule violation |

### Fields to Be Stored

```text
landing_g_force
landing_g_force_score
```

---

## 19. Arrival Threshold Distance

The software must calculate how far past the runway threshold the aircraft touched down.

### Proposed Rating

| Distance from threshold | Rating              |
| -----------------------: | -------------------- |
|                 150–600 m | Ideal                |
|                 600–900 m | Good                 |
|               900–1,200 m | Late                 |
|                  >1,200 m | Long landing         |
|                    <100 m | Very short / review  |

### Fields to Be Stored

```text
arrival_threshold_distance_m
arrival_threshold_distance_ft
arrival_threshold_score
```

---

## 20. Landing Bounces

The software must detect whether the aircraft lifts off again and touches down again after the first touchdown.

### Detection

A bounce occurs when:

* `on ground` briefly becomes true,
* then false again,
* then true again,
* and the groundspeed remains within the landing range.

### Fields to Be Stored

```text
landing_bounce_count
landing_first_touchdown_rate
landing_final_touchdown_rate
landing_worst_touchdown_rate
landing_first_g_force
landing_worst_g_force
```

### Proposed Rating

| Bounces | Rating             |
| ------: | ------------------ |
|       0 | Clean landing       |
|       1 | Slight bounce       |
|       2 | Multiple bounce     |
|      >2 | Rule violation      |

---

## 21. Takeoff METAR and Departure Runway

The software must store the METAR of the departure airport that is valid at, or closest in time to, the moment of departure.

### Fields to Be Stored

```text
departure_metar_raw
departure_metar_decoded_json
departure_metar_time
departure_runway_ident
departure_runway_heading
departure_wind_direction
departure_wind_speed
departure_crosswind_component
departure_headwind_component
departure_tailwind_component
departure_qnh
departure_visibility
departure_temperature
```

### Example

```text
Departure runway 25C detected.
METAR: EDDF 011020Z 25012KT 9999 FEW030 14/07 Q1016.
Crosswind component: 4 kt.
Headwind component: 11 kt.
```

---

## 22. Landing METAR and Arrival Runway

The software must store the METAR of the arrival airport that is valid at, or closest in time to, the moment of landing.

### Fields to Be Stored

```text
arrival_metar_raw
arrival_metar_decoded_json
arrival_metar_time
arrival_runway_ident
arrival_runway_heading
arrival_wind_direction
arrival_wind_speed
arrival_crosswind_component
arrival_headwind_component
arrival_tailwind_component
arrival_qnh
arrival_visibility
arrival_temperature
```

### Example

```text
Arrival runway 07R detected.
Touchdown with 9 kt crosswind and 3 kt headwind component.
```

---

## 23. Display in the PIREP

The phpVMS PIREP should display a dedicated section for the landing analysis.

### Example

```text
Landing Analysis

Arrival Runway: 25L
Touchdown Position: 50.036421, 8.543219
Landing Rate: -186 fpm
Landing G-Force: 1.42 G
Centerline Deviation: 7.4 m
Heading Deviation: 4.8°
Threshold Distance: 482 m
Landing Bounces: 0
Crosswind Component: 8 kt
Headwind Component: 4 kt
Arrival METAR: EDDF 011020Z 25012KT 9999 FEW030 14/07 Q1016
```

---

## 24. Custom Fields and Extensibility

The software must support custom fields.

### Examples

* Departure gate
* Arrival gate
* Passenger count
* Cargo weight
* Payload
* SimBrief OFP ID
* Online network
* VATSIM CID
* IVAO VID
* Aircraft mismatch flag
* Maintenance flags
* Disposable module data
* VA-specific additional data

### Requirement

Custom fields must be transmitted to phpVMS in a structured way and must not end up in the log only as free text.

---

## 25. Rule and Quality Evaluation

The software should evaluate the flight based on configurable rules.

### Must-Have Rules

* Taxi overspeed
* Takeoff overspeed
* Overspeed in flight
* Stall
* Slew mode
* Pause during the flight
* Simulation rate not equal to 1x
* Hard landing
* Very hard landing
* Landing bounce
* Wrong aircraft
* Wrong departure airport
* Wrong destination airport
* Wrong runway, optional
* Excessive centerline deviation
* Excessive heading deviation
* Landing too long
* Tailwind above threshold
* Crosswind above threshold

### Admin Configuration

All thresholds must be configurable in the admin area.

Examples:

```text
max_taxi_speed
max_landing_rate
max_landing_g_force
max_centerline_deviation
max_heading_deviation
max_threshold_distance
max_tailwind_component
max_crosswind_component
allow_sim_rate
allow_pause
allow_slew
```

---

## 26. Offline and Error Behavior

The software must be robust against connection problems.

### Must-Have

* Local buffering of the active flight
* Local storage of the flight log
* Local storage of position data
* Automatic resumption after client restart
* Retry queue for API requests
* No data loss during internet outage
* Clear error messages for API problems
* Debug export for support

### Should-Have

* Recovery after simulator crash
* Manual PIREP re-submission
* Local JSON export
* Support package with logs, client version, OS, simulator, and API responses

---

## 27. User Interface

The software should be modern, clear, and easy to use.

### Main Areas

* Login / VA configuration
* Dashboard
* Bid list
* Flight selection
* Flight details
* Aircraft selection
* SimBrief import
* Live status
* Flight log
* Landing analysis
* Settings
* Debug view
* PIREP review

### Must-Have

* Dark mode
* Display: phpVMS connected / not connected
* Display: simulator connected / not connected
* Display: current flight
* Display: flight phase
* Display: position
* Display: altitude
* Display: speed
* Display: fuel
* Display: distance to destination
* Start flight button
* End flight button
* Submit PIREP button
* Warning for aircraft mismatch
* Warning for wrong airport
* Warning for pause / slew / sim rate

---

## 28. Admin and Server Functions

An admin area should be provided on the phpVMS side.

### Must-Have

* Manage ACARS client versions
* Enforce minimum version
* Display update notices
* Configure tracking intervals
* Enable/disable rules
* Configure thresholds
* Configure allowed simulators
* Allow/forbid charter flights
* Allow/forbid aircraft mismatch
* Display API logs
* Display last client version per pilot
* Display last connection per pilot
* Enable/disable runway analysis
* Configure METAR source

### Should-Have

* VA branding
* Logo
* Colors
* Discord webhooks
* Notification on flight start
* Notification on landing
* Notification on PIREP
* Plugin system for custom rules

---

## 29. Security

### Must-Have

* Enforce HTTPS
* Store API key securely
* No plain-text passwords
* No sensitive data in logs
* Rate-limit handling
* Server-side validation of all critical values
* Plausibility check of route, distance, fuel, times, and landing rate
* Protection against manipulated PIREP data
* Check client version
* Protection against replay requests

### Should-Have

* Signed client updates
* Optional request signing
* Hash over flight log and position data
* Tamper detection
* Server-side anti-cheat rules

---

## 30. Update and Distribution Concept

### Client

* Installer for Windows
* Installer for macOS
* Automatic update check
* Update notice
* Optional mandatory updates
* Release notes
* Rollback capability

### Server Module

* Laravel/phpVMS-compliant
* Own migrations
* No core changes
* Theme-compatible
* Table-prefix-compatible
* Simple installation
* Simple uninstallation
* Updatable

---

## 31. Acceptance Criteria

The software is considered ready for acceptance when the following points are met:

1. Login with phpVMS URL and API key works.
2. Pilot data loads correctly.
3. Bids load correctly.
4. Flights can be loaded from phpVMS.
5. Aircraft data loads correctly.
6. Aircraft mismatch is detected.
7. MSFS 2020 is supported.
8. MSFS 2024 is supported.
9. X-Plane 11 is supported.
10. X-Plane 12 is supported.
11. Windows client works natively.
12. macOS client works natively.
13. Live tracking is sent to phpVMS.
14. Flight appears on the phpVMS live map.
15. Flight phases are detected automatically.
16. Flight log is generated completely.
17. Position data is stored.
18. Departure runway is detected.
19. Arrival runway is detected.
20. Takeoff METAR is stored.
21. Landing METAR is stored.
22. Landing rate is stored.
23. Landing G-force is stored.
24. Centerline deviation is calculated.
25. Heading deviation is calculated.
26. Threshold distance is calculated.
27. Landing bounces are detected.
28. Crosswind / headwind / tailwind are calculated.
29. PIREP is transmitted to phpVMS in full.
30. Custom fields are supported.
31. Network interruptions do not lead to data loss.
32. API errors are displayed in an understandable way.
33. Admin can configure rules and thresholds.
34. The solution works without changes to the phpVMS core.

---

## 32. Summary for Vendors / Developers

We are looking for a cross-platform ACARS client software for **phpVMS 7** that runs **natively on Windows and macOS** and supports at least **MSFS 2020, MSFS 2024, X-Plane 11, and X-Plane 12**.

The software should retrieve flights from phpVMS, display bids, verify aircraft, capture simulator data live, send position data to the website, produce a complete flight log, and transmit a complete PIREP to phpVMS after flight completion.

In addition, the software should perform a detailed **runway, takeoff, and landing analysis**. In doing so, departure runway, arrival runway, takeoff METAR, landing METAR, landing rate, landing G-force, centerline deviation, heading deviation, threshold distance, landing bounces, and wind components must be calculated and stored.

All data should be displayed in the PIREP, in the flight log, and optionally graphically on the website. Thresholds and rules must be configurable in the admin area. The solution should work without changes to the phpVMS core and be connected via a dedicated phpVMS module or a clean API integration.
