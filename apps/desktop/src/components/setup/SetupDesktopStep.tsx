import { useAppearance } from "../../theme/AppearanceProvider";
import type { DesktopStatus } from "../../types";

export function SetupDesktopStep({
  busy,
  certificates,
  onImportCaBundle,
}: {
  busy: boolean;
  certificates: DesktopStatus["additionalCaBundle"];
  onImportCaBundle: () => Promise<void>;
}) {
  const appearance = useAppearance();
  return (
    <div className="setup-desktop-options">
      <fieldset disabled={busy}>
        <legend>Theme</legend>
        <div className="setup-choice-group">
          {(["system", "light", "dark"] as const).map((theme) => (
            <label key={theme} className="setup-choice">
              <input
                type="radio"
                name="setup-theme"
                value={theme}
                checked={appearance.colorTheme === theme}
                onChange={() => appearance.setColorTheme(theme)}
              />
              <span
                className={`setup-theme-preview is-${theme}`}
                aria-hidden="true"
              >
                <i />
                <i />
              </span>
              <span>
                {theme === "system"
                  ? "System"
                  : theme === "light"
                    ? "Light"
                    : "Dark"}
              </span>
            </label>
          ))}
        </div>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Text size</legend>
        <div className="setup-choice-group setup-text-choices">
          {(["compact", "comfortable", "large"] as const).map((size) => (
            <label key={size} className="setup-choice">
              <input
                type="radio"
                name="setup-text-size"
                value={size}
                checked={appearance.textSize === size}
                onChange={() => appearance.setTextSize(size)}
              />
              <span>
                {size === "compact"
                  ? "Compact"
                  : size === "comfortable"
                    ? "Comfortable"
                    : "Large"}
              </span>
            </label>
          ))}
        </div>
      </fieldset>
      <p className="setup-hint">
        These preferences are saved for this app. You can change them later in
        Settings.
      </p>
      <details className="setup-certificates">
        <summary>Advanced: CA certificates</summary>
        <p>
          If your organization or a private server uses a custom certificate
          authority, import its CA bundle to let Colossus trust those
          connections.
        </p>
        <p>Importing applies immediately to Colossus on this computer.</p>
        {certificates.configured ? (
          <p role="status">
            {certificates.certificateCount} trusted{" "}
            {certificates.certificateCount === 1
              ? "certificate"
              : "certificates"}{" "}
            imported.
          </p>
        ) : null}
        <button
          type="button"
          className="button secondary"
          disabled={busy}
          onClick={() => void onImportCaBundle()}
        >
          {certificates.configured ? "Replace CA bundle" : "Import CA bundle"}
        </button>
      </details>
    </div>
  );
}
