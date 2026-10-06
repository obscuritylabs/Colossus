import type { ReactNode } from "react";
import colossusMark from "../../assets/colossus-mark.svg";

/** Application chrome using the same geometry as the shared settings frame. */
export function ApplicationFrame({
  title,
  navigation,
  current,
  onNavigate,
  sidebar,
  footer,
  header,
  children,
}: {
  title: string;
  navigation: readonly {
    id: string;
    label: string;
    icon: ReactNode;
    badge?: ReactNode;
  }[];
  current: string;
  onNavigate: (id: string) => void;
  sidebar?: ReactNode;
  footer?: ReactNode;
  header?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="managed-settings-shell application-frame">
      <a className="application-skip-link" href="#main-content">
        Skip to content
      </a>
      <header className="managed-settings-header">
        <div className="settings-brand">
          <img src={colossusMark} alt="" />
          <strong>Colossus</strong>
          <h1>{title}</h1>
        </div>
        <div className="application-header-actions">{header}</div>
      </header>
      <div className="settings-frame-layout">
        <aside className="settings-sidebar">
          <div className="settings-sidebar-content">
            {sidebar}
            <nav className="managed-settings-tabs" aria-label="Main navigation">
              {navigation.map((item) => (
                <button
                  type="button"
                  key={item.id}
                  className="sidebar-nav-item"
                  aria-current={current === item.id ? "page" : undefined}
                  onClick={() => onNavigate(item.id)}
                >
                  {item.icon}
                  <span>{item.label}</span>
                  {item.badge ? (
                    <span className="application-nav-badge">{item.badge}</span>
                  ) : null}
                </button>
              ))}
            </nav>
          </div>
          {footer ? (
            <div className="settings-sidebar-footer">{footer}</div>
          ) : null}
        </aside>
        <main className="settings-main" id="main-content" tabIndex={-1}>
          {children}
        </main>
      </div>
    </div>
  );
}
