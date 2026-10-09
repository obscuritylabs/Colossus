interface DesktopStartupProps {
  error?: string | undefined;
  onRetry?: () => void;
}

/** A neutral surface while the saved Desktop state is still unknown. */
export function DesktopStartup({ error, onRetry }: DesktopStartupProps) {
  return (
    <main className="desktop-startup" aria-busy={!error}>
      <div className="desktop-startup-content">
        {error ? (
          <>
            <h1>Couldn’t open Colossus</h1>
            <p role="alert">{error}</p>
            <button
              className="button secondary"
              type="button"
              onClick={onRetry}
            >
              Try again
            </button>
          </>
        ) : (
          <>
            <span className="desktop-startup-spinner" aria-hidden="true" />
            <p role="status">Starting Colossus…</p>
          </>
        )}
      </div>
    </main>
  );
}
