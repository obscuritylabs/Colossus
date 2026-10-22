import { Button, RadioGroup } from "@colossus/ui";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AppearanceProvider } from "./theme/AppearanceProvider";
import {
  DEFAULT_APPEARANCE,
  type ColorThemePreference,
  type DarkPalettePreference,
  type TextSizePreference,
} from "./theme/appearance";
import "./client-identity-review.css";

interface Review {
  uiToken: string;
  origin: string;
  candidates: { label: string; fingerprintSha256: string }[];
  appearance: {
    colorTheme: ColorThemePreference;
    darkPalette: DarkPalettePreference;
    textSize: TextSizePreference;
  };
}

export default function ClientIdentityReview() {
  const [review, setReview] = useState<Review | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    void invoke<Review>("client_identity_review_context")
      .then((value) => {
        if (active) setReview(value);
      })
      .catch(() => {
        if (active)
          setError("This certificate request is no longer available.");
      });
    return () => {
      active = false;
    };
  }, []);
  return review ? (
    <AppearanceProvider
      initialPreference={{ ...DEFAULT_APPEARANCE, ...review.appearance }}
    >
      <CertificateChoice review={review} />
    </AppearanceProvider>
  ) : (
    <main className="client-identity-review">
      {error ? (
        <p role="alert">{error}</p>
      ) : (
        <p role="status">Loading certificate request…</p>
      )}
    </main>
  );
}

function CertificateChoice({ review }: { review: Review }) {
  const [selected, setSelected] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const submitted = useRef(false);
  const dialog = useRef<HTMLElement>(null);

  async function respond(fingerprintSha256: string | null) {
    if (submitted.current) return;
    submitted.current = true;
    setSubmitting(true);
    try {
      await invoke("finish_client_identity_review", {
        uiToken: review.uiToken,
        fingerprintSha256,
      });
    } catch {
      setError("The request closed or changed. Check the site for the result.");
    }
  }

  function handleKeyDown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      void respond(null);
    } else if (event.key === "Tab") {
      const controls = Array.from(
        dialog.current?.querySelectorAll<HTMLButtonElement>(
          'button:not(:disabled):not([tabindex="-1"])',
        ) ?? [],
      );
      const first = controls[0];
      const last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    }
  }

  return (
    <main className="client-identity-review" onKeyDown={handleKeyDown}>
      <section
        ref={dialog}
        className="client-identity-review-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="certificate-heading"
        aria-describedby="certificate-scope"
      >
        <div className="client-identity-review-content">
          <p className="eyebrow">Browser authentication</p>
          <h1 id="certificate-heading">Choose a certificate</h1>
          <p className="client-identity-review-site">
            Site requesting your identity
            <strong>
              <bdi dir="ltr">{review.origin}</bdi>
            </strong>
          </p>
          <p id="certificate-scope">
            Choose the identity to share with this site for this connection.
            Cancel continues without a certificate.
          </p>
          <RadioGroup
            aria-label="Eligible certificates"
            value={selected}
            onValueChange={setSelected}
            disabled={submitting}
            options={review.candidates.map((candidate) => ({
              value: candidate.fingerprintSha256,
              label: candidate.label,
              description: (
                <span className="client-identity-review-fingerprint">
                  SHA-256 fingerprint
                  <code>
                    {candidate.fingerprintSha256.match(/.{2}/g)?.join(":")}
                  </code>
                </span>
              ),
            }))}
          />
          {error ? <p role="alert">{error}</p> : null}
        </div>
        <div className="client-identity-review-actions">
          <Button
            autoFocus
            variant="secondary"
            disabled={submitting}
            onClick={() => void respond(null)}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!selected || submitting}
            onClick={() => void respond(selected)}
          >
            {submitting ? "Applying choice…" : "Use certificate"}
          </Button>
        </div>
      </section>
    </main>
  );
}
