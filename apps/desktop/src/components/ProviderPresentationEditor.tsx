import { useEffect, useRef, useState } from "react";
import type { ProviderPresentation } from "../types";
import { MarkdownContent } from "./MarkdownContent";
import { BrowserLinkContext } from "./browser/BrowserLink";
import { ProviderIcon } from "./ProviderIcon";
import { openProviderInstructionsLink } from "../api";
import "./provider-presentation.css";
export const EMPTY_PRESENTATION: ProviderPresentation = {
  descriptionMarkdown: "",
  icon: null,
  darkIcon: null,
};
export const MAX_PROVIDER_DESCRIPTION_BYTES = 16_384;
export function providerDescriptionBytes(value: string) {
  return new TextEncoder().encode(value).length;
}
export function ProviderInstructions({
  resourceId,
  content,
}: {
  resourceId: string;
  content: string;
}) {
  const [error, setError] = useState("");
  return (
    <section
      className="provider-instructions"
      aria-label="Provider instructions"
    >
      <h4>Instructions</h4>
      <BrowserLinkContext.Provider
        value={(url, originalHref) => {
          setError("");
          void openProviderInstructionsLink(
            resourceId,
            originalHref ?? url,
          ).catch(() => setError("Could not open this instruction link."));
        }}
      >
        <MarkdownContent content={content} />
      </BrowserLinkContext.Provider>
      {error ? <p role="alert">{error}</p> : null}
    </section>
  );
}
export function ProviderPresentationEditor({
  value,
  busy,
  onChange,
  onReading,
}: {
  value: ProviderPresentation;
  busy: boolean;
  onChange: (value: ProviderPresentation) => void;
  onReading: (reading: boolean) => void;
}) {
  const [error, setError] = useState("");
  const [preview, setPreview] = useState(false);
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const bytes = providerDescriptionBytes(value.descriptionMarkdown);
  async function readIcon(file: File, field: "icon" | "darkIcon") {
    onReading(true);
    setError("");
    try {
      if (file.size > 65_536)
        throw new Error("Choose a PNG no larger than 64 KiB.");
      const data = new Uint8Array(await file.arrayBuffer());
      if (
        [137, 80, 78, 71, 13, 10, 26, 10].some(
          (byte, index) => data[index] !== byte,
        )
      )
        throw new Error("Choose a PNG image.");
      if (
        data.length < 24 ||
        String.fromCharCode(...data.slice(12, 16)) !== "IHDR"
      )
        throw new Error("Choose a valid PNG image.");
      const header = new DataView(
        data.buffer,
        data.byteOffset,
        data.byteLength,
      );
      if (header.getUint32(16) > 512 || header.getUint32(20) > 512)
        throw new Error("Choose an image no larger than 512 by 512 pixels.");
      const image = await createImageBitmap(file);
      const valid = image.width <= 512 && image.height <= 512;
      image.close();
      if (!valid)
        throw new Error("Choose an image no larger than 512 × 512 pixels.");
      if (!active.current) return;
      onChange({
        ...value,
        [field]: `data:image/png;base64,${btoa(Array.from(data, (byte) => String.fromCharCode(byte)).join(""))}`,
      });
    } catch (failure) {
      if (!active.current) return;
      setError(
        failure instanceof Error
          ? failure.message
          : "The image could not be read.",
      );
    } finally {
      if (active.current) onReading(false);
    }
  }
  return (
    <details className="provider-advanced-options">
      <summary>
        Advanced options <span>Instructions &amp; custom icons</span>
      </summary>
      <div className="provider-presentation-fields">
        <p>
          Add onboarding instructions and artwork for this provider. These
          travel with your exported Desktop setup file.
        </p>
        <div className="provider-description-heading">
          <label htmlFor="provider-description">
            Description &amp; instructions <small>Markdown</small>
          </label>
          <button
            type="button"
            className="text-button"
            onClick={() => setPreview(!preview)}
            aria-pressed={preview}
          >
            {preview ? "Edit Markdown" : "Preview"}
          </button>
        </div>
        {preview ? (
          <div
            className="provider-description-preview"
            aria-label="Description preview"
          >
            <BrowserLinkContext.Provider value={null}>
              <MarkdownContent
                content={
                  value.descriptionMarkdown ||
                  "Your instructions will appear here."
                }
              />
            </BrowserLinkContext.Provider>
          </div>
        ) : (
          <textarea
            id="provider-description"
            rows={6}
            disabled={busy}
            value={value.descriptionMarkdown}
            placeholder={
              "### Get an API token\nVisit [your team portal](https://example.com/tokens) to request access."
            }
            aria-describedby="provider-description-help"
            aria-invalid={bytes > MAX_PROVIDER_DESCRIPTION_BYTES}
            onChange={(event) =>
              onChange({ ...value, descriptionMarkdown: event.target.value })
            }
          />
        )}
        <small
          id="provider-description-help"
          className={
            bytes > MAX_PROVIDER_DESCRIPTION_BYTES
              ? "provider-presentation-error"
              : undefined
          }
        >
          {bytes.toLocaleString()} / 16,384 bytes · Use headings, lists, and
          links to explain how to connect.
        </small>
        <div className="provider-artwork-grid">
          {(["icon", "darkIcon"] as const).map((field) => (
            <div className={`provider-artwork is-${field}`} key={field}>
              <span className="provider-artwork-preview">
                <ProviderIcon
                  customIcon={value[field]}
                  customDarkIcon={null}
                  size={32}
                />
              </span>
              <div>
                <strong>
                  {field === "icon" ? "Provider icon" : "Dark theme icon"}
                </strong>
                <small>
                  {field === "icon"
                    ? "PNG · up to 512 × 512 · 64 KiB"
                    : "Optional; uses the provider icon when empty"}
                </small>
                <div className="provider-artwork-actions">
                  <label
                    className={`button secondary compact${busy ? " is-disabled" : ""}`}
                  >
                    {value[field] ? "Replace PNG" : "Choose PNG"}
                    <input
                      type="file"
                      accept="image/png,.png"
                      className="provider-icon-input"
                      disabled={busy}
                      aria-label={
                        field === "icon"
                          ? "Upload provider icon"
                          : "Upload dark theme icon"
                      }
                      onChange={(event) => {
                        const file = event.currentTarget.files?.[0];
                        event.currentTarget.value = "";
                        if (file) void readIcon(file, field);
                      }}
                    />
                  </label>
                  {value[field] ? (
                    <button
                      type="button"
                      className="text-button"
                      disabled={busy}
                      aria-label={
                        field === "icon"
                          ? "Remove provider icon"
                          : "Remove dark theme icon"
                      }
                      onClick={() => onChange({ ...value, [field]: null })}
                    >
                      Remove
                    </button>
                  ) : null}
                </div>
              </div>
            </div>
          ))}
        </div>
        {error ? (
          <p className="provider-presentation-error" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </details>
  );
}
