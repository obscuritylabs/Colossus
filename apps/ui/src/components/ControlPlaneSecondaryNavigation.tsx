import { IconChevronRight } from "@tabler/icons-react";
import type { MouseEvent, ReactNode } from "react";
import type { ControlPlaneNavigation } from "./ControlPlaneNavigationMenu.js";

export interface ControlPlaneSecondaryNavigationProps {
  /** Hosts change this key when the navigation scope changes. */
  id?: string;
  title: string;
  context: ReactNode;
  navigation: readonly ControlPlaneNavigation[];
  current: string;
  onNavigate: (id: string) => void;
}

function navigate(
  event: MouseEvent<HTMLAnchorElement>,
  item: ControlPlaneNavigation,
  onNavigate: (id: string) => void,
) {
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
}

/** A scoped, linkable second level; the host supplies every authorized destination. */
export function ControlPlaneSecondaryNavigation({
  title,
  context,
  navigation,
  current,
  onNavigate,
}: ControlPlaneSecondaryNavigationProps) {
  return (
    <div className="control-secondary-navigation">
      <header className="control-secondary-heading">
        <h2>{title}</h2>
        <div className="control-secondary-context">{context}</div>
      </header>
      <nav aria-label={`${title} sections`}>
        <ul>
          {navigation.map((item) => (
            <li key={item.id}>
              {item.href && !item.disabled ? (
                <a
                  className="control-secondary-link"
                  href={item.href}
                  aria-current={item.id === current ? "page" : undefined}
                  onClick={(event) => navigate(event, item, onNavigate)}
                >
                  <span aria-hidden="true">{item.icon}</span>
                  <span>{item.label}</span>
                </a>
              ) : (
                <button
                  type="button"
                  className="control-secondary-link"
                  disabled={item.disabled}
                  aria-current={item.id === current ? "page" : undefined}
                  onClick={() => onNavigate(item.id)}
                >
                  <span aria-hidden="true">{item.icon}</span>
                  <span>{item.label}</span>
                </button>
              )}
            </li>
          ))}
        </ul>
      </nav>
    </div>
  );
}

export function ControlPlaneNavigationBreadcrumb({
  title,
  navigation,
  current,
  onNavigate,
}: ControlPlaneSecondaryNavigationProps) {
  const first = navigation[0];
  const selected = navigation.find((item) => item.id === current);
  return (
    <nav className="control-navigation-breadcrumb" aria-label="Breadcrumb">
      <ol>
        <li>
          {first?.href && !first.disabled ? (
            <a
              href={first.href}
              onClick={(event) => navigate(event, first, onNavigate)}
            >
              {title}
            </a>
          ) : (
            <span>{title}</span>
          )}
        </li>
        {selected ? (
          <li>
            <IconChevronRight size={14} aria-hidden="true" />
            <span aria-current="page">{selected.label}</span>
          </li>
        ) : null}
      </ol>
    </nav>
  );
}
