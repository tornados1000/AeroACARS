// Glossar-Modal für RunwayDiagramV2.
// Spec: docs/spec/runway-diagram-v2.contract.md §Glossar (17 Begriffe).
// Accessible: ESC schließt, Focus-Trap auf Modal, role="dialog".

import { useEffect, useRef } from "react";

interface GlossaryEntry {
  abbr: string;
  full: string;
  explanation: string;
}

const ENTRIES: GlossaryEntry[] = [
  {
    abbr: "Threshold (THR)",
    full: "Bahnschwelle",
    explanation:
      "Die großen weißen Querstreifen am Bahnanfang. Ab dieser Linie darfst du landen.",
  },
  {
    abbr: "Touchdown (TD)",
    full: "Aufsetzen",
    explanation: "Der Moment, in dem die Räder den Bahnbelag berühren.",
  },
  {
    abbr: "Centerline (CL)",
    full: "Mittellinie",
    explanation: "Die gestrichelte weiße Linie genau in der Mitte der Bahn.",
  },
  {
    abbr: "Centerline-Offset / XTD",
    full: "Seitenabweichung",
    explanation:
      "Wie weit links oder rechts von der Mittellinie bist du aufgesetzt? Idealwert: 0 m.",
  },
  {
    abbr: "TDZ — Touchdown Zone",
    full: "Aufsetzzone",
    explanation:
      "Der Soll-Bereich zum Aufsetzen: erste 900 m der Bahn oder das erste Drittel (was kürzer ist). Auf echten Bahnen siehst du sie als Gruppen weißer Querstreifen.",
  },
  {
    abbr: "AIM — Aim Point",
    full: "Ziel-Markierung",
    explanation:
      "Zwei große weiße Quadrate auf der Bahn — das sind die Aiming-Point-Marken. Im stabilisierten Anflug zielt dein Blick GENAU dort hin, weil der 3°-Glideslope dich exakt zu diesem Punkt führen würde, wenn du nicht abfangen (flaren) würdest. Beim Flare hebst du die Nase, drosselst — und setzt typisch 50–150 m HINTER dem Aim-Point auf (= Anfang der TDZ). Position laut ICAO Annex 14: 400 m hinter der Schwelle bei Bahnen ≥ 2400 m, 300 m bei 1500–2399 m, 250 m bei 1200–1499 m.",
  },
  {
    abbr: "TCH — Threshold Crossing Height",
    full: "Schwellen-Überflug-Höhe",
    explanation:
      "Wie hoch warst du über dem Boden, als du die Schwelle überflogen hast? ILS-Anflug typisch 49 ft (≈ 15 m). Zu niedrig: Tail-Strike-Risiko. Zu hoch: Long-Landing.",
  },
  {
    abbr: "DDS — Displaced Threshold",
    full: "Versetzte Schwelle",
    explanation:
      "Manche Bahnen haben einen Bereich VOR der echten Landeschwelle, der für die Landung verboten ist (Pfeile auf der Bahn). Aufsetzen davor = illegal. Beispiel: OLBA RWY 35, 820 m DDS.",
  },
  {
    abbr: "Glide Slope",
    full: "Anflug-Winkel",
    explanation: "ILS-Standard 3°. Du sinkst 1 m für je 19 m vorwärts.",
  },
  {
    abbr: "Rollout",
    full: "Ausrollstrecke",
    explanation:
      "Wie viele Meter rollst du nach dem Aufsetzen, bis du auf ~40 kt abgebremst hast — das ist die typische High-Speed-Exit-Geschwindigkeit, mit der du am nächsten Rollwege-Abzweig die Bahn verlässt. Bis zum vollen Stand auf der Bahn rollt fast niemand aus (das wäre verschwendete Bahn).",
  },
  {
    abbr: "Bahn-Auslastung",
    full: "",
    explanation: "Ausrollstrecke ÷ Bahnlänge × 100 %. 80 % = nur 20 % Bahn übrig (knapp).",
  },
  {
    abbr: "AIRAC-Cycle",
    full: "",
    explanation:
      'Offizielle Aviation-Daten werden alle 28 Tage aktualisiert. „Cycle 2604" = 4. Update 2026.',
  },
  {
    abbr: "VPS Navdata",
    full: "",
    explanation:
      "Zentrale, vom VA-Admin gepflegte AIRAC-Daten auf dem VPS. Pilot-Client zieht sie pro Flugstart. Technische Quelle dahinter: Aerosoft DFD (Lizenz: VA-Admin-Subscription).",
  },
  {
    abbr: "OurAirports",
    full: "",
    explanation:
      "Community-Wiki-Datenquelle als Fallback wenn der VPS nicht erreichbar ist. Schwellen-Positionen können abweichen.",
  },
  {
    abbr: "AGL",
    full: "Above Ground Level",
    explanation: "Höhe über Grund (nicht über Meer).",
  },
  {
    abbr: "fpm",
    full: "Feet per Minute",
    explanation: "Sinkrate-Einheit. Negativ = Sinkflug.",
  },
  {
    abbr: "kt",
    full: "Knots / Knoten",
    explanation: "Geschwindigkeitseinheit, ≈ 1.852 km/h.",
  },
];

export function GlossaryModal({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeBtnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    closeBtnRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== "Tab") return;
      const root = dialogRef.current;
      if (!root) return;
      const focusables = root.querySelectorAll<HTMLElement>(
        'button, [href], input, [tabindex]:not([tabindex="-1"])',
      );
      if (focusables.length === 0) return;
      const first = focusables[0]!;
      const last = focusables[focusables.length - 1]!;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      onClick={onClose}
      role="presentation"
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.65)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 10000,
        padding: 16,
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="rwy-glossary-title"
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "#111827",
          border: "1px solid rgba(255,255,255,0.18)",
          borderRadius: 10,
          maxWidth: 760,
          width: "100%",
          maxHeight: "85vh",
          display: "flex",
          flexDirection: "column",
          boxShadow: "0 20px 60px rgba(0,0,0,0.6)",
        }}
      >
        <header
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "14px 18px",
            borderBottom: "1px solid rgba(255,255,255,0.10)",
          }}
        >
          <h3 id="rwy-glossary-title" style={{ margin: 0, fontSize: "1.1rem" }}>
            🛬 Begriffe in der Landebahn-Analyse
          </h3>
          <button
            ref={closeBtnRef}
            type="button"
            onClick={onClose}
            aria-label="Glossar schließen"
            style={{
              padding: "4px 12px",
              background: "rgba(255,255,255,0.08)",
              border: "1px solid rgba(255,255,255,0.18)",
              borderRadius: 6,
              color: "inherit",
              cursor: "pointer",
            }}
          >
            Schließen ✕
          </button>
        </header>
        <div
          style={{
            padding: "12px 18px 18px 18px",
            overflowY: "auto",
            display: "flex",
            flexDirection: "column",
            gap: 14,
          }}
        >
          <p style={{ margin: 0, opacity: 0.75, fontSize: "0.88rem" }}>
            Kurzerklärung aller Begriffe und Abkürzungen, die im Diagramm und in
            den Detail-Karten auftauchen — in einfacher Sprache.
          </p>
          {ENTRIES.map((e) => (
            <div
              key={e.abbr}
              style={{
                background: "rgba(255,255,255,0.04)",
                border: "1px solid rgba(255,255,255,0.08)",
                borderRadius: 6,
                padding: "10px 12px",
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "baseline",
                  gap: 8,
                  flexWrap: "wrap",
                  marginBottom: 4,
                }}
              >
                <strong style={{ fontSize: "0.98rem" }}>{e.abbr}</strong>
                {e.full && (
                  <span style={{ opacity: 0.6, fontSize: "0.85rem" }}>
                    — {e.full}
                  </span>
                )}
              </div>
              <div style={{ fontSize: "0.9rem", lineHeight: 1.5, opacity: 0.92 }}>
                {e.explanation}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
