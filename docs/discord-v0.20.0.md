# Discord-Entwurf — AeroACARS v0.20.0

> Entwurf. Noch NICHT gepostet. Kanal: vermutlich #ankündigungen.
> An zwei Stellen ist ein Platzhalter: der Name des Melders (falls du ihn nennen willst)
> und der Kanal-Verweis. Sonst kann der Text so raus.

---

**✈️ AeroACARS v0.20.0 ist da — „eine Landung, eine Bewertung"**

Einer von euch hat gemeldet, dass derselbe Touchdown **zwei verschiedene Sinkraten** anzeigt: die Landing-Karte sagte **−206 fpm**, die ACARS-Log-Zeile direkt daneben **−233 fpm**. Danke dafür — die Meldung hat deutlich mehr aufgedeckt als den einen Wert.

**Was wirklich los war**

Es war kein Rechenfehler, sondern ein Muster: **fast jede Zahl im Landungs-Tab wurde an zwei oder drei Stellen unabhängig berechnet** — einmal in der App, einmal noch mal fürs Log, manchmal ein drittes Mal für die Live-Karte. Welche Zahl ihr gesehen habt, hing davon ab, wohin ihr geschaut habt. Deshalb haben sich frühere Korrekturen auch immer wie Flickwerk angefühlt: wir haben jedes Mal ein Symptom kuriert, nie die Ursache.

**Was wir gemacht haben**

Die App bewertet eure Landung jetzt **genau einmal**, friert das Ergebnis ein, und jede Anzeige zeigt nur noch dieses eine Ergebnis — Landungs-Tab, ACARS-Log, PIREP, Live-Karte, PDF.

Beim Aufräumen sind noch fünf Widersprüche aufgefallen, die **niemand gemeldet hatte**:

• **G-Kraft stand doppelt im Tab** — die Kachel zeigte den Rohwert, der Balken daneben den geglätteten.
• **Note und Klasse widersprachen sich** — im PIREP stand „A (smooth) — 92/100" neben „A+ (SMOOTH, 100/100)". Außerdem passten die Grenzen nicht zusammen: 47 Punkte ergaben „F (firm)", 88 Punkte „A (acceptable)".
• **Die Bahn-Auslastung wurde dreifach gerechnet** — und das Bahn-Diagramm hat die Piste bei versetzter Schwelle sogar **zu lang gezeichnet**.
• **Die Anflug-Stabilität zählte eure absichtlichen Kurven im Anflug als „Unruhe"** — je nachdem, welche Anzeige ihr angeschaut habt.
• **Der „Bestwert"** wurde auf einem Rohwert sortiert und kürte dadurch bei alten Flügen die falsche beste Landung.

Und die berühmten **109 kg** aus dem Loadsheet: die Kachel zeigte die **SimBrief-Planung**, das Log die **echte Messung**. Kein Fehler — das ist schlicht der Taxi-Sprit. Die Kachel sagt jetzt „Plan" dazu.

**Was das für euch heißt**

• **Eure Punkte können sich leicht verschieben.** Die Anflug-Stabilität rechnet jetzt mit dem sauberen Wert, und die Klassen-Wörter liegen jetzt auf den Notengrenzen. Die Formeln selbst haben wir nicht angefasst.
• **Bereits geflogene Landungen werden nicht neu berechnet.** Sie behalten ihre Zahlen.
• Bei **alten** Landungen kann sich im Web-Dashboard das Kategorie-Wort ändern: es beschreibt jetzt die Gesamtbewertung (inkl. Anflug, Bahn, Sprit) statt nur des Aufsetzens. Eine butterweiche Landung nach einem wackligen Anflug las sich vorher als „SMOOTH · 62".

**Damit es nicht zurückkommt**

Die Regel „es gibt nur eine Bewertung" ist jetzt nicht mehr nur ein Kommentar im Code, sondern wird **erzwungen**: ein Test schlägt fehl, sobald eine Anzeige wieder anfängt, selbst zu rechnen. Genau daran ist es letztes Mal gescheitert — die Regel stand da, und trotzdem hat sich jemand drübergesetzt.

**Update:** startet AeroACARS, das Update wird euch angeboten.

*Und bitte weiter so mit den Meldungen. Ein Screenshot mit zwei Zahlen drauf hat hier mehr gefunden als jeder Testlauf.*
