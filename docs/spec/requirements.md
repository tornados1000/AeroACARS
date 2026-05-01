# Anforderungsspezifikation — CloudeAcars

**Version:** 1.0 (Kickoff)
**Datum:** 2026-05-01
**Sprache:** Deutsch (Originaldokument des Auftraggebers)

> Dieses Dokument ist die ungekürzte Originalspezifikation, wie sie zum Projektkickoff vom Auftraggeber übergeben wurde. Es ist die normative Quelle für Phasen 1–5. Änderungen werden über ADRs ([`../decisions/`](../decisions/)) und Protokoll-/Architektur-Updates nachgehalten, **nicht** durch Edit dieser Datei.

---

## 1. Ziel der Software

Es soll eine moderne ACARS-Client-Software entwickelt werden, die mit **phpVMS 7** kommuniziert, Flüge automatisch aufzeichnet und Live-Informationen an die Webseite überträgt.

Die Software soll ähnlich wie **vmsACARS** oder **smartCARS** funktionieren, jedoch mit besonderem Fokus auf:

* native Unterstützung für **Windows und macOS**
* Unterstützung von **MSFS 2020, MSFS 2024, X-Plane 11 und X-Plane 12**
* direkte Kommunikation mit **phpVMS 7**
* Live-Tracking auf der Webseite
* vollständige Flugaufzeichnung
* automatische PIREP-Übermittlung
* detaillierte Start-, Runway- und Landing-Analyse
* Speicherung von Fluglog, Positionsdaten, METAR, Landing Rate, G-Force, Runway-Abweichungen und weiteren Qualitätsdaten

---

## 2. Unterstützte Betriebssysteme

### Muss-Anforderung

Die ACARS-Software muss als native Desktop-Software für folgende Systeme verfügbar sein:

| Betriebssystem | Anforderung                    |
| -------------- | ------------------------------ |
| Windows        | Muss unterstützt werden        |
| macOS          | Muss nativ unterstützt werden  |
| Linux          | Optional / spätere Erweiterung |

Die macOS-Version darf nicht nur über eine Windows-VM oder Wine funktionieren. Es wird eine echte native macOS-Unterstützung erwartet.

---

## 3. Unterstützte Simulatoren

Die Software muss mindestens folgende Simulatoren unterstützen:

| Simulator                  | Version | Anforderung |
| -------------------------- | ------: | ----------- |
| Microsoft Flight Simulator |    2020 | Muss        |
| Microsoft Flight Simulator |    2024 | Muss        |
| X-Plane                    |      11 | Muss        |
| X-Plane                    |      12 | Muss        |

Optional sollte die Architektur später für weitere Simulatoren wie Prepar3D oder FSX erweiterbar sein.

---

## 4. Grundfunktion

Die Software soll dem Piloten ermöglichen, einen Flug aus phpVMS zu laden, im Simulator zu starten, den Flug live aufzuzeichnen und nach Abschluss automatisch oder manuell einen PIREP an phpVMS zu senden.

### Muss-Funktionen

* Login mit phpVMS-URL und API-Key
* Abruf von Pilotendaten
* Abruf von Bids
* Abruf von Flugplänen
* Abruf von Flotten- und Aircraft-Daten
* Auswahl eines Fluges
* Auswahl oder Prüfung des Flugzeugs
* Verbindung zum Simulator
* Start der Flugaufzeichnung
* Live-Übertragung an phpVMS
* Erstellung eines Flight Logs
* Aufzeichnung von Positionsdaten
* Erkennung von Flugphasen
* Übermittlung eines vollständigen PIREPs
* Speicherung von Custom Fields
* Fehler- und Offline-Handling

---

## 5. phpVMS 7 Integration

Die Software muss mit phpVMS 7 über eine saubere API-Kommunikation verbunden werden.

### Muss-Anforderungen

* Kommunikation ausschließlich über HTTPS
* Authentifizierung per phpVMS API-Key
* Nutzung von JSON für API-Requests und Responses
* korrekte Behandlung von API-Fehlern
* Unterstützung von Rate Limits
* keine Speicherung von Zugangsdaten im Klartext
* sichere Speicherung des API-Keys im Betriebssystem
* Kompatibilität mit phpVMS 7
* Kompatibilität mit phpVMS Table Prefix
* keine Änderungen am phpVMS-Core, soweit möglich
* Bereitstellung eines eigenen phpVMS-Moduls oder API-Connectors

---

## 6. Datenabruf aus phpVMS

Die ACARS-Software muss folgende Daten aus phpVMS abrufen können:

### Pilotendaten

* Name
* Pilot ID
* Airline
* Heimatflughafen
* aktueller Standort
* Rank
* Status
* Berechtigungen
* vorhandene Bids

### Flugplandaten

* Airline
* Flugnummer
* Route Code
* Route Leg
* Departure Airport
* Arrival Airport
* Alternate Airport
* geplante Route
* geplante Flugzeit
* Distanz
* Flight Type
* Cruise Altitude
* SimBrief-Informationen, sofern vorhanden

### Aircraft- und Fleet-Daten

* Aircraft ID
* Registration / Tail Number
* ICAO Type
* Airline
* Subfleet
* aktueller Standort
* Status
* allowed / not allowed für den Flug
* Maintenance-Status, sofern vorhanden
* Blocked / In Use Status, sofern vorhanden

---

## 7. Flugvorbereitung im Client

Vor dem Start soll der Pilot alle wichtigen Informationen prüfen und vorbereiten können.

### Muss-Funktionen

* Bid auswählen
* Flug aus phpVMS-Flugplan laden
* optional Charterflug erstellen, sofern erlaubt
* Aircraft auswählen
* Aircraft-Mismatch erkennen
* Departure / Arrival / Alternate prüfen
* Route anzeigen
* SimBrief OFP importieren oder abrufen
* Cruise Altitude anzeigen oder setzen
* Block Fuel erfassen
* Payload / ZFW erfassen
* geplante Flugzeit anzeigen
* geplante Distanz anzeigen

### Soll-Funktionen

* Import von Flugplandateien
* Anzeige der Route auf einer Karte
* Anzeige von Wetter / METAR
* Warnung bei falschem Startflughafen
* Warnung bei falschem Flugzeug
* Warnung bei fehlendem SimBrief-Plan

---

## 8. Simulator-Daten und Telemetrie

Die Software muss während des Fluges kontinuierlich Daten aus dem Simulator erfassen.

### Mindestdaten

* Latitude
* Longitude
* Altitude MSL
* Altitude AGL
* Heading
* Groundspeed
* IAS
* TAS
* Vertical Speed
* Pitch
* Bank
* On Ground Status
* Parking Brake
* Gear Status
* Flaps Position
* Engine Status
* Fuel Quantity
* Fuel Used
* Payload
* ZFW
* G-Force
* Touchdown G-Force
* Landing Rate
* Stall Warning
* Overspeed Warning
* Pause Status
* Slew Mode
* Simulation Rate
* Squawk
* COM/NAV Frequencies
* Wind Direction
* Wind Speed
* QNH
* Aircraft Type
* Simulator Version

---

## 9. Flugphasen

Die Software muss Flugphasen automatisch erkennen und protokollieren.

### Mindestphasen

* Preflight
* Boarding
* Pushback
* Taxi Out
* Takeoff Roll
* Takeoff
* Climb
* Cruise
* Descent
* Approach
* Final
* Landing
* Taxi In
* Blocks On
* Arrived
* PIREP Submitted

### Beispiel für Flight Log

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

## 10. Live-Tracking zur Webseite

Während des Fluges muss die Software regelmäßig Live-Daten an phpVMS senden.

### Muss-Daten für Live Map

* Pilot ID
* Pilot Name
* Callsign
* Flight Number
* Aircraft Registration
* Aircraft ICAO
* Departure Airport
* Arrival Airport
* Latitude
* Longitude
* Altitude
* Groundspeed
* Heading
* Vertical Speed
* Flight Phase
* Distance flown
* Distance remaining
* Estimated Time Enroute
* Estimated Time Remaining
* Route Progress
* Last Update Timestamp
* Online Network, sofern erkannt

### Anforderungen

* Aktualisierung in konfigurierbaren Intervallen
* keine Überlastung der phpVMS-API
* automatische Wiederverbindung bei Verbindungsabbruch
* Speicherung nicht gesendeter Positionsdaten in einer lokalen Queue
* Darstellung des Fluges auf der phpVMS-Livemap

---

## 11. Flight Log / Ereignisaufzeichnung

Die Software muss ein vollständiges Flight Log erzeugen.

### Muss-Ereignisse

* ACARS gestartet
* Verbindung zu phpVMS hergestellt
* Simulator erkannt
* Flug geladen
* Aircraft erkannt
* Aircraft-Mismatch erkannt
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
* Top of Descent
* Approach
* Final
* Touchdown
* Landing Rate
* Landing G-Force
* Bounce Detection
* Taxi In
* Blocks On
* Flight Ended
* PIREP Submitted

### Soll

* Regelbasierte Events
* konfigurierbare Grenzwerte
* Debounce / Timeout gegen Log-Spam
* Admin-Konfiguration für Events

---

## 12. PIREP-Übermittlung

Nach Flugende muss ein vollständiger PIREP an phpVMS übertragen werden.

### Muss-Felder

* Airline
* Flight Number
* Route Code
* Route Leg
* Departure Airport
* Arrival Airport
* Alternate Airport
* Aircraft ID
* Aircraft Registration
* Aircraft ICAO
* Flight Time
* Block Time
* Taxi Out Time
* Taxi In Time
* Planned Distance
* Actual Distance
* Planned Route
* Actual Route
* Cruise Altitude
* ZFW
* Payload
* Block Fuel
* Fuel Used
* Remaining Fuel
* Landing Rate
* Landing G-Force
* Flight Log
* ACARS Positions
* Source Name
* Simulator
* Client Version
* Custom Fields
* Raw Data / Debug Data optional

---

## 13. Runway-, Takeoff- und Landing-Analyse

Die Software soll eine detaillierte Analyse von Start und Landung durchführen.

Dafür wird eine geeignete Runway-Datenbasis benötigt. Diese muss mindestens Runway-Kennung, Koordinaten, Heading, Länge, Breite und Threshold-Positionen enthalten.

### Benötigte Runway-Daten

* Airport ICAO
* Runway Ident
* Runway True Heading
* Runway Magnetic Heading
* Runway Length
* Runway Width
* Runway Start Threshold Latitude / Longitude
* Runway End Latitude / Longitude
* Displaced Threshold, falls vorhanden
* Elevation
* Surface Type

---

## 14. Departure Runway Detection

Die Software muss automatisch erkennen, von welcher Runway gestartet wurde.

### Erkennung anhand von

* Position beim Takeoff Roll
* Position beim Liftoff
* Aircraft Heading beim Start
* nächstgelegener Runway Centerline
* Entfernung zur Runway
* Departure Airport aus dem Flugplan
* Runway-Heading-Toleranz

### Zu speichernde Felder

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

Die Software muss automatisch erkennen, auf welcher Runway gelandet wurde.

### Erkennung anhand von

* Touchdown-Position
* Aircraft Heading beim Touchdown
* nächstgelegener Runway Centerline
* Entfernung zum Threshold
* Entfernung zur Runway Centerline
* Arrival Airport aus dem Flugplan
* Groundspeed
* On Ground Transition
* Runway-Heading-Toleranz

### Zu speichernde Felder

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

Die Software muss berechnen, wie weit das Flugzeug beim Touchdown seitlich von der Runway-Centerline entfernt war.

### Einheit

* Meter
* optional Feet

### Bewertungsvorschlag

| Abweichung | Bewertung      |
| ---------: | -------------- |
|      0–5 m | Sehr gut       |
|     5–10 m | Gut            |
|    10–20 m | Akzeptabel     |
|    20–35 m | Hinweis        |
|      >35 m | Rule Violation |

### Zu speichernde Felder

```text
arrival_centerline_deviation_m
arrival_centerline_deviation_ft
arrival_centerline_score
```

---

## 17. Arrival Heading Deviation

Die Software muss berechnen, wie stark der Flugzeugkurs beim Touchdown vom Runway-Heading abweicht.

### Bewertungsvorschlag

| Abweichung | Bewertung      |
| ---------: | -------------- |
|       0–3° | Sehr gut       |
|       3–6° | Gut            |
|      6–10° | Akzeptabel     |
|     10–15° | Hinweis        |
|       >15° | Rule Violation |

### Zu speichernde Felder

```text
arrival_heading_deviation_deg
arrival_heading_score
```

---

## 18. Landing G-Force

Die Software muss die G-Force beim Touchdown speichern.

### Bewertungsvorschlag

|     G-Force | Bewertung                     |
| ----------: | ----------------------------- |
|     <1.30 G | Sehr weich                    |
| 1.30–1.60 G | Normal                        |
| 1.60–1.90 G | Hart                          |
| 1.90–2.20 G | Sehr hart                     |
|     >2.20 G | Hard Landing / Rule Violation |

### Zu speichernde Felder

```text
landing_g_force
landing_g_force_score
```

---

## 19. Arrival Threshold Distance

Die Software muss berechnen, wie weit hinter dem Runway Threshold das Flugzeug aufgesetzt hat.

### Bewertungsvorschlag

| Distanz ab Threshold | Bewertung          |
| -------------------: | ------------------ |
|            150–600 m | Ideal              |
|            600–900 m | Gut                |
|          900–1.200 m | Spät               |
|             >1.200 m | Long Landing       |
|               <100 m | Sehr kurz / prüfen |

### Zu speichernde Felder

```text
arrival_threshold_distance_m
arrival_threshold_distance_ft
arrival_threshold_score
```

---

## 20. Landing Bounces

Die Software muss erkennen, ob das Flugzeug nach dem ersten Touchdown erneut abhebt und wieder aufsetzt.

### Erkennung

Ein Bounce liegt vor, wenn:

* `on ground` kurzzeitig true wird,
* danach wieder false,
* danach erneut true,
* und die Groundspeed weiterhin im Landing-Bereich liegt.

### Zu speichernde Felder

```text
landing_bounce_count
landing_first_touchdown_rate
landing_final_touchdown_rate
landing_worst_touchdown_rate
landing_first_g_force
landing_worst_g_force
```

### Bewertungsvorschlag

| Bounces | Bewertung         |
| ------: | ----------------- |
|       0 | Saubere Landung   |
|       1 | Leichter Bounce   |
|       2 | Mehrfacher Bounce |
|      >2 | Rule Violation    |

---

## 21. Takeoff METAR und Departure Runway

Die Software muss zum Zeitpunkt des Starts das gültige oder zeitlich nächstliegende METAR des Departure Airports speichern.

### Zu speichernde Felder

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

### Beispiel

```text
Departure runway 25C detected.
METAR: EDDF 011020Z 25012KT 9999 FEW030 14/07 Q1016.
Crosswind component: 4 kt.
Headwind component: 11 kt.
```

---

## 22. Landing METAR und Arrival Runway

Die Software muss zum Zeitpunkt der Landung das gültige oder zeitlich nächstliegende METAR des Arrival Airports speichern.

### Zu speichernde Felder

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

### Beispiel

```text
Arrival runway 07R detected.
Touchdown with 9 kt crosswind and 3 kt headwind component.
```

---

## 23. Darstellung im PIREP

Im phpVMS-PIREP soll ein eigener Bereich für die Landing-Analyse angezeigt werden.

### Beispiel

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

## 24. Custom Fields und Erweiterbarkeit

Die Software muss Custom Fields unterstützen.

### Beispiele

* Departure Gate
* Arrival Gate
* Passenger Count
* Cargo Weight
* Payload
* SimBrief OFP ID
* Online Network
* VATSIM CID
* IVAO VID
* Aircraft Mismatch Flag
* Maintenance Flags
* Disposable Module Daten
* VA-spezifische Zusatzdaten

### Anforderung

Custom Fields müssen strukturiert an phpVMS übertragen werden können und dürfen nicht nur als Freitext im Log landen.

---

## 25. Regel- und Qualitätsbewertung

Die Software soll den Flug anhand konfigurierbarer Regeln bewerten.

### Muss-Regeln

* Taxi Overspeed
* Takeoff Overspeed
* Overspeed im Flug
* Stall
* Slew Mode
* Pause während des Fluges
* Simulation Rate ungleich 1x
* harte Landung
* sehr harte Landung
* Landing Bounce
* falsches Flugzeug
* falscher Startflughafen
* falscher Zielflughafen
* falsche Runway optional
* zu hohe Centerline Deviation
* zu hohe Heading Deviation
* zu lange Landung
* Tailwind über Grenzwert
* Crosswind über Grenzwert

### Admin-Konfiguration

Alle Grenzwerte müssen im Adminbereich konfigurierbar sein.

Beispiele:

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

## 26. Offline- und Fehlerverhalten

Die Software muss robust gegen Verbindungsprobleme sein.

### Muss

* lokale Zwischenspeicherung des aktiven Fluges
* lokale Speicherung des Flight Logs
* lokale Speicherung der Positionsdaten
* automatische Wiederaufnahme nach Client-Neustart
* Retry-Queue für API-Requests
* kein Datenverlust bei Internetabbruch
* klare Fehlermeldungen bei API-Problemen
* Debug-Export für Support

### Soll

* Wiederherstellung nach Simulator-Crash
* manuelle PIREP-Nachreichung
* lokaler JSON-Export
* Support-Paket mit Logs, Client-Version, OS, Simulator und API-Antworten

---

## 27. Benutzeroberfläche

Die Software soll modern, übersichtlich und einfach bedienbar sein.

### Hauptbereiche

* Login / VA-Konfiguration
* Dashboard
* Bid-Liste
* Flugauswahl
* Flugdetails
* Aircraft-Auswahl
* SimBrief-Import
* Live-Status
* Flight Log
* Landing Analysis
* Einstellungen
* Debug-Ansicht
* PIREP Review

### Muss

* Dark Mode
* Anzeige: phpVMS verbunden / nicht verbunden
* Anzeige: Simulator verbunden / nicht verbunden
* Anzeige: aktueller Flug
* Anzeige: Flugphase
* Anzeige: Position
* Anzeige: Höhe
* Anzeige: Geschwindigkeit
* Anzeige: Fuel
* Anzeige: Distanz zum Ziel
* Start Flight Button
* End Flight Button
* Submit PIREP Button
* Warnung bei Aircraft-Mismatch
* Warnung bei falschem Airport
* Warnung bei Pause / Slew / Simrate

---

## 28. Admin- und Serverfunktionen

Auf phpVMS-Seite soll ein Adminbereich bereitgestellt werden.

### Muss

* ACARS-Client Versionen verwalten
* Mindestversion erzwingen
* Update-Hinweise anzeigen
* Tracking-Intervalle konfigurieren
* Regeln aktivieren/deaktivieren
* Grenzwerte konfigurieren
* erlaubte Simulatoren konfigurieren
* Charterflüge erlauben/verbieten
* Aircraft-Mismatch erlauben/verbieten
* API-Logs anzeigen
* letzte Client-Version je Pilot anzeigen
* letzte Verbindung je Pilot anzeigen
* Runway-Analyse aktivieren/deaktivieren
* METAR-Quelle konfigurieren

### Soll

* VA-Branding
* Logo
* Farben
* Discord Webhooks
* Benachrichtigung bei Flugstart
* Benachrichtigung bei Landung
* Benachrichtigung bei PIREP
* Plugin-System für eigene Regeln

---

## 29. Sicherheit

### Muss

* HTTPS erzwingen
* API-Key sicher speichern
* keine Passwörter im Klartext
* keine sensiblen Daten in Logs
* Rate-Limit-Handling
* Servervalidierung aller kritischen Werte
* Plausibilitätsprüfung von Route, Distanz, Fuel, Zeiten und Landing Rate
* Schutz gegen manipulierte PIREP-Daten
* Client-Version prüfen
* Schutz vor Replay-Requests

### Soll

* signierte Client-Updates
* optionale Request-Signierung
* Hash über Flight Log und Positionsdaten
* Manipulationserkennung
* serverseitige Anti-Cheat-Regeln

---

## 30. Update- und Verteilungskonzept

### Client

* Installer für Windows
* Installer für macOS
* automatischer Update-Check
* Update-Hinweis
* optionale Pflichtupdates
* Release Notes
* Rollback-Möglichkeit

### Servermodul

* Laravel/phpVMS-konform
* eigene Migrationen
* keine Core-Änderungen
* theme-kompatibel
* table-prefix-kompatibel
* einfache Installation
* einfache Deinstallation
* updatefähig

---

## 31. Abnahmekriterien

Die Software gilt als abnahmefähig, wenn folgende Punkte erfüllt sind:

1. Login mit phpVMS-URL und API-Key funktioniert.
2. Pilotendaten werden korrekt geladen.
3. Bids werden korrekt geladen.
4. Flüge können aus phpVMS geladen werden.
5. Aircraft-Daten werden korrekt geladen.
6. Aircraft-Mismatch wird erkannt.
7. MSFS 2020 wird unterstützt.
8. MSFS 2024 wird unterstützt.
9. X-Plane 11 wird unterstützt.
10. X-Plane 12 wird unterstützt.
11. Windows-Client funktioniert nativ.
12. macOS-Client funktioniert nativ.
13. Live-Tracking wird an phpVMS gesendet.
14. Flug erscheint auf der phpVMS-Livemap.
15. Flugphasen werden automatisch erkannt.
16. Flight Log wird vollständig erzeugt.
17. Positionsdaten werden gespeichert.
18. Departure Runway wird erkannt.
19. Arrival Runway wird erkannt.
20. Takeoff METAR wird gespeichert.
21. Landing METAR wird gespeichert.
22. Landing Rate wird gespeichert.
23. Landing G-Force wird gespeichert.
24. Centerline Deviation wird berechnet.
25. Heading Deviation wird berechnet.
26. Threshold Distance wird berechnet.
27. Landing Bounces werden erkannt.
28. Crosswind / Headwind / Tailwind werden berechnet.
29. PIREP wird vollständig an phpVMS übertragen.
30. Custom Fields werden unterstützt.
31. Netzwerkunterbrechungen führen nicht zu Datenverlust.
32. API-Fehler werden verständlich angezeigt.
33. Admin kann Regeln und Grenzwerte konfigurieren.
34. Die Lösung funktioniert ohne Änderungen am phpVMS-Core.

---

## 32. Kurzfassung für Anbieter / Entwickler

Gesucht wird eine plattformübergreifende ACARS-Client-Software für **phpVMS 7**, die unter **Windows und macOS nativ** läuft und mindestens **MSFS 2020, MSFS 2024, X-Plane 11 und X-Plane 12** unterstützt.

Die Software soll Flüge aus phpVMS abrufen, Bids anzeigen, Flugzeuge prüfen, Simulator-Daten live erfassen, Positionsdaten an die Webseite senden, ein vollständiges Flight Log erzeugen und nach Flugende einen vollständigen PIREP an phpVMS übermitteln.

Zusätzlich soll die Software eine detaillierte **Runway-, Takeoff- und Landing-Analyse** durchführen. Dabei müssen Departure Runway, Arrival Runway, Takeoff METAR, Landing METAR, Landing Rate, Landing G-Force, Centerline Deviation, Heading Deviation, Threshold Distance, Landing Bounces sowie Windkomponenten berechnet und gespeichert werden.

Alle Daten sollen im PIREP, im Flight Log und optional grafisch auf der Webseite dargestellt werden. Die Grenzwerte und Regeln müssen im Adminbereich konfigurierbar sein. Die Lösung soll ohne Änderungen am phpVMS-Core funktionieren und über ein eigenes phpVMS-Modul oder eine saubere API-Integration angebunden werden.
