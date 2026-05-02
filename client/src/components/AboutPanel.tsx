import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { AppInfo } from "../types";

/**
 * About / Credits tab. Quiet, dezent, but acknowledges every project /
 * dataset / piece of reverse-engineering AeroACARS stands on. Each
 * line is a real reference — `OurAirports`, `BeatMyLanding`, `GEES`,
 * `vmsACARS`, `LandingToast` — these were studied in detail to get
 * the touchdown analyzer right.
 *
 * Renders the Gifhorn credit prominently but not loudly. Pilot opens
 * this tab when they want to know "what is this thing made of"; it
 * isn't shoved in their face on every other screen.
 */
export function AboutPanel() {
  const { t } = useTranslation();
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const ai = await invoke<AppInfo>("app_info");
        if (!cancelled) setInfo(ai);
      } catch {
        // app_info should never fail; if it does, just leave the
        // hero strip blank rather than showing a confusing error.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="about">
      <header className="about__hero">
        <h2 className="about__title">AeroACARS</h2>
        <p className="about__tagline">{t("about.tagline")}</p>
        {info && (
          <p className="about__version">
            v{info.version}
            {info.commit ? <> · <code>{info.commit.slice(0, 7)}</code></> : null}
          </p>
        )}
        {info && <p className="about__credit">{info.credit}</p>}
      </header>

      <div className="about__section">
        <h3>{t("about.purpose_title")}</h3>
        <p>{t("about.purpose_body")}</p>
      </div>

      <div className="about__section">
        <h3>{t("about.acknowledgements_title")}</h3>
        <p className="about__hint">{t("about.acknowledgements_intro")}</p>
        <ul className="about__list">
          <li>
            <strong>OurAirports</strong> — Public-domain runway dataset
            powering the centerline/threshold correlation.{" "}
            <a
              href="https://ourairports.com/data/"
              target="_blank"
              rel="noreferrer"
            >
              ourairports.com/data
            </a>
          </li>
          <li>
            <strong>BeatMyLanding</strong> — Reference for touchdown-
            window timings (500 ms / 1500 ms) and bounce-detection
            calibration via AGL edges.
          </li>
          <li>
            <strong>GEES</strong> — Open-source landing-rate logger;
            confirmed our V/S sign convention and native sideslip
            via VEL_BODY_X/Z.{" "}
            <a
              href="https://github.com/scelts/gees"
              target="_blank"
              rel="noreferrer"
            >
              github.com/scelts/gees
            </a>
          </li>
          <li>
            <strong>LandingToast</strong> — Validated the live-VS-at-
            on-ground-edge approach (no PLANE TOUCHDOWN NORMAL VELOCITY
            needed).
          </li>
          <li>
            <strong>vmsACARS</strong> — Reference for phpVMS API
            patterns (correct bid-delete endpoint, ACARS PHP module
            routes).
          </li>
          <li>
            <strong>Tauri 2 + React + Rust</strong> — App framework.
          </li>
          <li>
            <strong>Microsoft Flight Simulator SDK</strong> — SimConnect
            client API.
          </li>
          <li>
            <strong>Laminar Research X-Plane SDK</strong> — UDP RREF
            DataRef protocol documentation.
          </li>
        </ul>
      </div>

      <div className="about__section">
        <h3>{t("about.thresholds_title")}</h3>
        <p className="about__hint">{t("about.thresholds_intro")}</p>
        <ul className="about__list">
          <li>Boeing 737 FCOM — Hard-Landing inspection trigger</li>
          <li>Airbus A320 FCOM — TD sink rate / inspection criteria</li>
          <li>Lufthansa FOQA — Public category bands</li>
          <li>vmsACARS rules.yml — Default 500 fpm hard-landing rule</li>
        </ul>
      </div>

      <footer className="about__footer">
        <p>© {new Date().getFullYear()} AeroACARS Project · MIT License</p>
        <p>{info?.credit}</p>
      </footer>
    </section>
  );
}
