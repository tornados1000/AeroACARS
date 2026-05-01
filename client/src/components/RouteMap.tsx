import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface AirportInfo {
  icao: string;
  name: string;
  lat: number;
  lon: number;
}

interface Props {
  dptIcao: string;
  arrIcao: string;
  currentLat?: number | null;
  currentLon?: number | null;
}

/**
 * Mini route map: departure pin on the left, arrival pin on the right,
 * a soft accent line connecting them, and a small plane glyph that
 * tracks the live position along the route. Coordinates come from the
 * existing `airport_get` Tauri command (cached in the Rust state, no
 * round-trip when the dashboard already looked the airport up for the
 * arrival-distance check).
 *
 * The projection is intentionally simple — straight-line interpolation
 * between the two endpoints in lat/lon space, then mapped to the SVG
 * viewbox. For typical short/medium-haul VA flights this looks correct;
 * a Mercator-projected great circle is a Phase-M follow-up if VAs ever
 * need accurate transatlantic shapes.
 */
export function RouteMap({ dptIcao, arrIcao, currentLat, currentLon }: Props) {
  const [dpt, setDpt] = useState<AirportInfo | null>(null);
  const [arr, setArr] = useState<AirportInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function fetchOne(icao: string, set: (a: AirportInfo) => void) {
      try {
        const data = await invoke<AirportInfo>("airport_get", { icao });
        if (!cancelled) set(data);
      } catch {
        // Silent — the dashboard already shows the ICAO without coords.
      }
    }
    if (dptIcao) void fetchOne(dptIcao, setDpt);
    if (arrIcao) void fetchOne(arrIcao, setArr);
    return () => {
      cancelled = true;
    };
  }, [dptIcao, arrIcao]);

  if (!dpt || !arr) {
    return null;
  }

  // Project lat/lon onto a 0..1 axis between the two endpoints.
  // `t = 0` is the departure, `t = 1` is the arrival; t < 0 / > 1 is
  // clamped so the plane sits on the route even if the sim coordinates
  // are slightly off (taxi positions, post-landing rollout).
  const dx = arr.lon - dpt.lon;
  const dy = arr.lat - dpt.lat;
  const lenSq = dx * dx + dy * dy;
  let progress = 0;
  if (lenSq > 0 && currentLat != null && currentLon != null) {
    const px = currentLon - dpt.lon;
    const py = currentLat - dpt.lat;
    progress = (px * dx + py * dy) / lenSq;
    progress = Math.max(0, Math.min(1, progress));
  }

  // SVG layout — fixed viewbox so the percentages are stable.
  const W = 600;
  const H = 80;
  const padX = 40;
  const trackY = H / 2;
  const dptX = padX;
  const arrX = W - padX;
  const planeX = dptX + (arrX - dptX) * progress;

  return (
    <div className="route-map" aria-hidden="true">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        {/* Track — soft accent line between the two pins. */}
        <line
          x1={dptX}
          y1={trackY}
          x2={arrX}
          y2={trackY}
          className="route-map__track"
        />
        {/* Travelled portion — solid accent. */}
        <line
          x1={dptX}
          y1={trackY}
          x2={planeX}
          y2={trackY}
          className="route-map__track-fill"
        />
        {/* Departure pin */}
        <circle cx={dptX} cy={trackY} r={6} className="route-map__pin" />
        <text
          x={dptX}
          y={trackY - 14}
          textAnchor="middle"
          className="route-map__icao"
        >
          {dpt.icao}
        </text>
        {/* Arrival pin */}
        <circle cx={arrX} cy={trackY} r={6} className="route-map__pin" />
        <text
          x={arrX}
          y={trackY - 14}
          textAnchor="middle"
          className="route-map__icao"
        >
          {arr.icao}
        </text>
        {/* Plane glyph — sits on the progress point. Translate keeps
         *  the path centered on planeX so it looks attached to the line. */}
        <g
          transform={`translate(${planeX} ${trackY})`}
          className="route-map__plane"
        >
          <circle r={10} className="route-map__plane-bg" />
          <path
            d="M 8 0 L -7 -5 L -3 0 L -7 5 Z"
            className="route-map__plane-shape"
          />
        </g>
      </svg>
    </div>
  );
}
