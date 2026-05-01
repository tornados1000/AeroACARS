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
 * a soft accent line between them, and a small plane glyph that
 * tracks the live position along the route.
 *
 * Implementation: HTML/CSS layout (not SVG) so the pin circles and
 * plane glyph stay perfectly round regardless of container width.
 * The previous SVG version with preserveAspectRatio="none" stretched
 * everything horizontally, producing oval pins on wide screens.
 *
 * Projection is intentionally simple — straight-line interpolation
 * between the two endpoints in lat/lon space. Correct enough for
 * short/medium-haul; a true Mercator great circle is a follow-up if
 * VAs need accurate transatlantic shapes.
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
  // Clamped so taxi positions and post-touchdown rollout keep the
  // plane on the route instead of overshooting.
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

  // Track padding (matches CSS) — the pins sit `--track-padding` from
  // each end, the plane / fill use the same offsets.
  const trackProgressPct = progress * 100;

  return (
    <div className="route-map">
      <div className="route-map__track">
        <div className="route-map__line" />
        <div
          className="route-map__line-fill"
          style={{ width: `${trackProgressPct}%` }}
        />

        <div className="route-map__pin route-map__pin--dpt">
          <span className="route-map__icao">{dpt.icao}</span>
          <span className="route-map__dot" />
        </div>

        <div className="route-map__pin route-map__pin--arr">
          <span className="route-map__icao">{arr.icao}</span>
          <span className="route-map__dot" />
        </div>

        <div
          className="route-map__plane"
          style={{ left: `${trackProgressPct}%` }}
          aria-hidden="true"
        >
          <svg viewBox="0 0 24 24" width="20" height="20">
            <path
              fill="currentColor"
              d="M21 16v-2l-8-5V3.5a1.5 1.5 0 0 0-3 0V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5z"
            />
          </svg>
        </div>
      </div>
    </div>
  );
}
