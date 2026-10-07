import type { ReactNode } from "react";
import colossusMark from "../../assets/colossus-mark.svg";
export interface ControlPlaneNavigation {
  id: string;
  label: string;
  shortLabel?: string;
  href?: string;
  icon: ReactNode;
  disabled?: boolean;
}
export interface ClassificationBanner {
  enabled: boolean;
  text: string;
  tone: "neutral" | "info" | "warning" | "danger";
  position: "top" | "top_and_bottom";
}
export function ControlPlaneFrame({
  current,
  navigation,
  onNavigate,
  sidebar,
  header,
  footer,
  children,
  classification,
  user,
}: {
  current: string;
  navigation: ControlPlaneNavigation[];
  onNavigate: (id: string) => void;
  sidebar?: ReactNode;
  header?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  classification?: ClassificationBanner | undefined;
  user?: ReactNode;
}) {
  const banner = classification?.enabled ? (
    <div
      className={`control-classification classification-${classification.tone}`}
      role="note"
      aria-label="Environment classification"
    >
      {classification.text}
    </div>
  ) : null;
  return (
    <div
      className={`control-plane-frame ${sidebar ? "has-scope-sidebar" : ""}`}
    >
      <a href="#control-main" className="application-skip-link">
        Skip to content
      </a>
      {banner}
      <div className="control-frame-layout">
        <aside
          className="control-primary-rail"
          aria-label="Control Plane navigation"
        >
          <div className="control-brand" title="Colossus Control Plane">
            <img src={colossusMark} alt="Colossus" />
          </div>
          <nav>
            {navigation.map((item) =>
              item.href && !item.disabled ? (
                <a
                  key={item.id}
                  href={item.href}
                  className={current === item.id ? "is-active" : ""}
                  aria-current={current === item.id ? "page" : undefined}
                  title={item.label}
                  aria-label={item.label}
                  onClick={(event) => {
                    if (
                      event.button ||
                      event.metaKey ||
                      event.ctrlKey ||
                      event.shiftKey ||
                      event.altKey
                    )
                      return;
                    event.preventDefault();
                    onNavigate(item.id);
                  }}
                >
                  {item.icon}
                  <span>{item.shortLabel ?? item.label}</span>
                </a>
              ) : (
                <button
                  key={item.id}
                  type="button"
                  className={current === item.id ? "is-active" : ""}
                  aria-current={current === item.id ? "page" : undefined}
                  disabled={item.disabled}
                  onClick={() => onNavigate(item.id)}
                  title={item.label}
                  aria-label={item.label}
                >
                  {item.icon}
                  <span>{item.shortLabel ?? item.label}</span>
                </button>
              ),
            )}
          </nav>
          <div className="control-rail-footer">
            {user}
            {footer}
          </div>
        </aside>
        {sidebar ? (
          <aside className="control-scope-sidebar" aria-label="Selected scope">
            {sidebar}
          </aside>
        ) : null}
        <div className="control-frame-main">
          <header className="control-header">
            <strong>Control Plane</strong>
            <div>{header}</div>
          </header>
          <main id="control-main" tabIndex={-1}>
            {children}
          </main>
        </div>
      </div>
      {classification?.enabled && classification.position === "top_and_bottom"
        ? banner
        : null}
    </div>
  );
}
