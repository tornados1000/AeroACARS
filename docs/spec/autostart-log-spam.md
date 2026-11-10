# Spec — Auto-Start-Log-Spam dämpfen

**Status:** ✅ behoben in v0.12.11 · **Aufwand:** klein · **Priorität:** mittel

> Umgesetzt: der unbedingte `*g = None`-Reset bei erfüllten Auto-Start-
> Voraussetzungen ist entfernt (er löschte den Throttle-State vor dem
> „no_bids"-Check), und der „keine Bids"-Hinweis nutzt jetzt reine
> Edge-Detektion (loggt nur beim Wechsel auf `no_bids`).

## Problem

Wenn am Boden kein phpVMS-Bid gebucht ist (oder der Pilot nicht
eingeloggt ist), schreibt die Auto-Start-Logik **alle ~8 Sekunden**
denselben Eintrag ins ACARS-Aktivitätsprotokoll:

```
Auto-Start: keine Bids verfuegbar
Im phpVMS-Bid-Tab steht aktuell nichts gebucht. Logged-in?
```

Beleg: `activity_log.json` von Sven M / FDX1636 — von 16:04 bis 16:48
über **150 identische Einträge** in Folge. Das Protokoll wird unbrauchbar
(echte Ereignisse gehen unter) und der 1000-Einträge-Ring läuft schnell
voll.

## Ziel

Die wiederholte „keine Bids"-Meldung **nicht** bei jedem Auto-Start-Poll
ins Aktivitätsprotokoll schreiben.

## Ansatz (Optionen)

- **Edge-Detektion:** den Eintrag nur schreiben, wenn sich der Zustand
  *ändert* (z. B. von „Bids vorhanden" → „keine Bids", oder beim ersten
  Auftreten). Der Activity-Log hat bereits einen Change-Detector für
  Avionik-Werte — gleiche Idee hier anwenden.
- **Throttle:** denselben Auto-Start-Hinweis höchstens alle N Minuten
  (z. B. 1×/10 min) protokollieren.
- **Severity:** Auto-Start-Polling-Ergebnisse ggf. gar nicht ins
  Piloten-Protokoll, sondern nur ins technische Log.

Empfehlung: Edge-Detektion + zusätzlich ein Throttle als Sicherheitsnetz.

## Scope / Non-Goals

- **In scope:** reine Client-Änderung an der Auto-Start-Logik /
  Activity-Log-Schreibstelle.
- **Out of scope:** die Auto-Start-Funktion selbst, phpVMS-Login-Handling.
