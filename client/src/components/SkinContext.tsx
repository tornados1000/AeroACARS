// SkinContext — React-Context der den aktuellen V2-Skin liefert.
//
// Lade-Strategie (Pilot-Client):
//   1. Beim Provider-Mount: bundled DEFAULT_SKIN sofort verwenden
//      (synchrones initial render — keine flash of unstyled content)
//   2. Im Hintergrund (await): VPS-Endpoint /api/v2-skin fetchen
//   3. Bei Erfolg: localStorage cachen + State aktualisieren
//   4. Bei Fehler (offline): aus localStorage Cache laden (falls vorhanden)
//      sonst bleibt es bei DEFAULT_SKIN
//
// Webapp: identisch, kann denselben Provider importieren.

import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import {
  DEFAULT_SKIN,
  mergeWithDefaults,
  type V2Skin,
} from "./runwayV2Skin";

const CACHE_KEY = "aeroacars.v2-skin.cache.v1";

const SkinContext = createContext<V2Skin>(DEFAULT_SKIN);

export function useV2Skin(): V2Skin {
  return useContext(SkinContext);
}

interface SkinProviderProps {
  children: ReactNode;
  /** VPS-Endpoint. Default: live.nexusairva.org. Für Tests/Dev überschreibbar. */
  endpoint?: string;
  /**
   * Auth-Token (Bearer). Optional — wenn der VPS-Endpoint Pilot-Auth
   * verlangt, muss hier ein Token rein. Wenn der Endpoint public ist,
   * kann es weggelassen werden.
   */
  authToken?: string | null;
}

export function SkinProvider({
  children,
  endpoint = "https://live.nexusairva.org/api/v2-skin",
  authToken,
}: SkinProviderProps) {
  // Initial: Cache aus localStorage probieren, sonst DEFAULT.
  const [skin, setSkin] = useState<V2Skin>(() => {
    try {
      const cached = localStorage.getItem(CACHE_KEY);
      if (cached) {
        const parsed = JSON.parse(cached) as Partial<V2Skin>;
        return mergeWithDefaults(parsed);
      }
    } catch {
      /* localStorage-Fehler ignorieren, fallback auf default */
    }
    return DEFAULT_SKIN;
  });

  useEffect(() => {
    const ac = new AbortController();
    (async () => {
      try {
        const headers: Record<string, string> = { Accept: "application/json" };
        if (authToken) {
          headers["Authorization"] = `Bearer ${authToken}`;
        }
        const r = await fetch(endpoint, {
          signal: ac.signal,
          headers,
        });
        if (!r.ok) {
          // 404 = noch kein Skin auf VPS deployed, kein Fehler
          if (r.status !== 404) {
            console.warn(`v2-skin fetch failed: HTTP ${r.status}`);
          }
          return;
        }
        const json = (await r.json()) as Partial<V2Skin>;
        const merged = mergeWithDefaults(json);
        setSkin(merged);
        try {
          localStorage.setItem(CACHE_KEY, JSON.stringify(merged));
        } catch {
          /* quota-error o.ä. — kein Caching, kein Drama */
        }
      } catch (err) {
        if ((err as DOMException)?.name === "AbortError") return;
        // Offline oder Network-Error → bleib beim Cache/Default
        console.info("v2-skin fetch unavailable (using cache/default):", err);
      }
    })();
    return () => ac.abort();
  }, [endpoint, authToken]);

  return <SkinContext.Provider value={skin}>{children}</SkinContext.Provider>;
}
