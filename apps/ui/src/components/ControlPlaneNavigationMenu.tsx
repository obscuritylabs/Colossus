import { useEffect, useId, useState, type ReactNode } from "react";
import { IconChevronRight } from "@tabler/icons-react";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "./ui/sidebar.js";

export interface ControlPlaneNavigation {
  id: string;
  label: string;
  shortLabel?: string;
  href?: string;
  icon: ReactNode;
  disabled?: boolean;
  group?: string;
}

const STATIC_GROUPS: readonly string[] = [];

/** Hosts supply authorized destinations; this menu owns disclosure presentation only. */
export function ControlPlaneNavigationMenu({
  current,
  navigation,
  onNavigate,
  collapsibleNavigationGroups = STATIC_GROUPS,
}: {
  current: string;
  navigation: readonly ControlPlaneNavigation[];
  onNavigate: (id: string) => void;
  collapsibleNavigationGroups?: readonly string[];
}) {
  const groups = new Map<string, ControlPlaneNavigation[]>();
  for (const item of navigation) {
    const name = item.group ?? "";
    const group = groups.get(name);
    if (group) group.push(item);
    else groups.set(name, [item]);
  }
  const collapsible = new Set(collapsibleNavigationGroups);
  return (
    <nav aria-label="Application">
      {[...groups].map(([name, items]) => (
        <NavigationGroup
          key={name}
          name={name}
          items={items}
          current={current}
          onNavigate={onNavigate}
          collapsible={Boolean(name) && collapsible.has(name)}
        />
      ))}
    </nav>
  );
}

function NavigationGroup({
  name,
  items,
  current,
  onNavigate,
  collapsible,
}: {
  name: string;
  items: readonly ControlPlaneNavigation[];
  current: string;
  onNavigate: (id: string) => void;
  collapsible: boolean;
}) {
  const { open, setOpen, isMobile } = useSidebar();
  const rail = !isMobile && !open;
  const active = items.some((item) => item.id === current);
  const [expanded, setExpanded] = useState(active);
  const contentId = useId();
  useEffect(() => {
    // A new destination can arrive through a bookmark or another navigation
    // surface. Reveal it without overriding a later explicit collapse.
    if (active) setExpanded(true);
  }, [active, current]);
  const disclosed = !collapsible || (!rail && expanded);
  return (
    <section
      className="control-navigation-group"
      aria-label={name || undefined}
    >
      {name ? (
        collapsible ? (
          <h2 className="control-navigation-disclosure-heading">
            <button
              type="button"
              className="control-navigation-disclosure"
              aria-label={name}
              title={name}
              aria-expanded={disclosed}
              aria-controls={contentId}
              data-active={active || undefined}
              onClick={() => {
                if (rail) {
                  setExpanded(true);
                  setOpen(true);
                } else setExpanded((value) => !value);
              }}
            >
              <span
                className="control-navigation-disclosure-icon"
                aria-hidden="true"
              >
                {items[0]?.icon ?? <IconChevronRight size={16} />}
              </span>
              <span className="control-navigation-disclosure-label">
                {name}
              </span>
              <IconChevronRight
                className="control-navigation-disclosure-chevron"
                size={14}
                aria-hidden="true"
              />
            </button>
          </h2>
        ) : (
          <h2 className="control-navigation-heading">{name}</h2>
        )
      ) : null}
      <SidebarMenu
        id={contentId}
        className="control-navigation-children"
        hidden={!disclosed}
      >
        {items.map((item) => (
          <SidebarMenuItem key={item.id}>
            <SidebarMenuButton
              asChild
              isActive={current === item.id}
              className="control-app-link"
              tooltip={rail ? item.label : undefined}
            >
              {item.href && !item.disabled ? (
                <a
                  href={item.href}
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
                  <span>{item.label}</span>
                </a>
              ) : (
                <button
                  type="button"
                  aria-current={current === item.id ? "page" : undefined}
                  disabled={item.disabled}
                  onClick={() => onNavigate(item.id)}
                  title={item.label}
                  aria-label={item.label}
                >
                  {item.icon}
                  <span>{item.label}</span>
                </button>
              )}
            </SidebarMenuButton>
          </SidebarMenuItem>
        ))}
      </SidebarMenu>
    </section>
  );
}
