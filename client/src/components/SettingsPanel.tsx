import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { SimKind } from "../types";
import type { Theme } from "../theme";

const ALL_KINDS: SimKind[] = [
  "msfs2024",
  "msfs2020",
  "xplane11",
  "xplane12",
  "off",
];

interface Props {
  debugMode: boolean;
  onDebugModeChange: (next: boolean) => void;
  theme: Theme;
  onThemeChange: (next: Theme) => void;
}

export function SettingsPanel({
  debugMode,
  onDebugModeChange,
  theme,
  onThemeChange,
}: Props) {
  const { t, i18n } = useTranslation();
  const [kind, setKind] = useState<SimKind | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const k = await invoke<string>("sim_get_kind");
        if (!cancelled) setKind(k as SimKind);
      } catch {
        if (!cancelled) setKind("off");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleKindChange(next: SimKind) {
    if (busy) return;
    setBusy(true);
    setKind(next);
    try {
      await invoke("sim_set_kind", { kind: next });
    } catch {
      // ignore
    } finally {
      setBusy(false);
    }
  }

  const language = i18n.resolvedLanguage ?? "en";

  return (
    <section className="settings">
      <header className="settings__header">
        <h2>{t("settings.title")}</h2>
        <p className="settings__hint">{t("settings.description")}</p>
      </header>

      <div className="settings__section">
        <h3>{t("settings.appearance_section")}</h3>

        <label className="settings__field">
          <span className="settings__field-label">
            {t("settings.language_label")}
          </span>
          <select
            value={language}
            onChange={(e) => i18n.changeLanguage(e.target.value)}
          >
            <option value="de">{t("actions.language_de")}</option>
            <option value="en">{t("actions.language_en")}</option>
          </select>
        </label>

        <label className="settings__field">
          <span className="settings__field-label">
            {t("settings.theme_label")}
          </span>
          <select
            value={theme}
            onChange={(e) => onThemeChange(e.target.value as Theme)}
          >
            <option value="dark">{t("settings.theme_dark")}</option>
            <option value="light">{t("settings.theme_light")}</option>
          </select>
        </label>
      </div>

      <div className="settings__section">
        <h3>{t("settings.simulator_section")}</h3>
        <p className="settings__row-hint">{t("settings.simulator_hint")}</p>
        <label className="settings__field">
          <span className="settings__field-label">
            {t("settings.simulator_label")}
          </span>
          <select
            value={kind ?? "off"}
            onChange={(e) => handleKindChange(e.target.value as SimKind)}
            disabled={busy || kind === null}
          >
            {ALL_KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`sim.kinds.${k}`)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="settings__section">
        <h3>{t("settings.developer_section")}</h3>
        <label className="settings__checkbox">
          <input
            type="checkbox"
            checked={debugMode}
            onChange={(e) => onDebugModeChange(e.target.checked)}
          />
          <span>
            <strong>{t("settings.debug_mode_label")}</strong>
            <span className="settings__row-hint">
              {t("settings.debug_mode_hint")}
            </span>
          </span>
        </label>
      </div>
    </section>
  );
}
