import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useSyncExternalStore,
  type AnchorHTMLAttributes,
  type ReactNode,
} from "react";
import { parseRoute, safeReturnPath, type Route } from "./routes";

const changed = "colossus:web:navigation";
const returnKey = "colossus:web:sign-in-return";
const snapshot = () => `${window.location.pathname}${window.location.search}`;
function subscribe(listener: () => void) {
  window.addEventListener("popstate", listener);
  window.addEventListener(changed, listener);
  return () => {
    window.removeEventListener("popstate", listener);
    window.removeEventListener(changed, listener);
  };
}
function historyMarker() {
  const marker = window.history.state?.colossusRoute;
  return marker && typeof marker.from === "string" ? marker : null;
}
export function navigate(href: string, replace = false) {
  const path = safeReturnPath(href, window.location.origin);
  if (!path) throw new Error("Invalid Control Plane destination.");
  if (path === snapshot()) return;
  const state = {
    colossusRoute: {
      from: replace ? (historyMarker()?.from ?? null) : snapshot(),
    },
  };
  if (replace) window.history.replaceState(state, "", path);
  else window.history.pushState(state, "", path);
  window.dispatchEvent(new Event(changed));
}
export function navigateBack(fallback: string) {
  const from = historyMarker()?.from;
  if (from && safeReturnPath(from, window.location.origin))
    window.history.back();
  else navigate(fallback, true);
}
/** The OIDC callback may return Home. Only a validated local route is retained. */
export function rememberSignInReturn() {
  const path = safeReturnPath(snapshot(), window.location.origin);
  try {
    if (path)
      window.sessionStorage.setItem(
        returnKey,
        JSON.stringify({ path, expires: Date.now() + 600000 }),
      );
    else window.sessionStorage.removeItem(returnKey);
  } catch {
    /* Storage may be unavailable. */
  }
}
export function restoreSignInReturn() {
  try {
    const raw = window.sessionStorage.getItem(returnKey);
    window.sessionStorage.removeItem(returnKey);
    if (!raw || snapshot() !== "/") return;
    const value = JSON.parse(raw) as { path?: unknown; expires?: unknown };
    if (
      typeof value.path === "string" &&
      typeof value.expires === "number" &&
      value.expires > Date.now() &&
      value.expires <= Date.now() + 600000 &&
      safeReturnPath(value.path, window.location.origin)
    )
      navigate(value.path, true);
  } catch {
    /* Invalid or unavailable storage must not affect sign-in. */
  }
}
interface Navigation {
  route: Route;
  href: string;
  go: (href: string, replace?: boolean) => void;
  back: (fallback: string) => void;
}
const Context = createContext<Navigation | null>(null);
export function NavigationProvider({ children }: { children: ReactNode }) {
  const href = useSyncExternalStore(subscribe, snapshot, snapshot);
  const route = parseRoute(href, window.location.origin);
  const go = useCallback(
    (path: string, replace = false) => navigate(path, replace),
    [],
  );
  const back = useCallback((path: string) => navigateBack(path), []);
  useEffect(() => {
    document.getElementById("control-main")?.focus();
  }, [href]);
  return <Context value={{ route, href, go, back }}>{children}</Context>;
}
export function useNavigation() {
  const value = useContext(Context);
  if (!value) throw new Error("Navigation must be supplied by the web host.");
  return value;
}
export function RouteLink({
  href,
  onNavigate,
  back = false,
  children,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  onNavigate?: () => void;
  back?: boolean;
  children: ReactNode;
}) {
  const navigation = useContext(Context);
  return (
    <a
      {...props}
      href={href}
      onClick={(event) => {
        props.onClick?.(event);
        if (
          event.defaultPrevented ||
          event.button !== 0 ||
          event.metaKey ||
          event.ctrlKey ||
          event.shiftKey ||
          event.altKey ||
          props.target ||
          props.download !== undefined
        )
          return;
        if (navigation) {
          event.preventDefault();
          if (back) navigation.back(href);
          else navigation.go(href);
        } else if (onNavigate) {
          event.preventDefault();
          onNavigate();
        }
      }}
    >
      {children}
    </a>
  );
}
