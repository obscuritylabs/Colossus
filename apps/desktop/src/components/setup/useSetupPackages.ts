import { useEffect, useState } from "react";
import { CommandFailure, listSetupPackages } from "../../api";
import { SETUP_CHANGED_EVENT, type SetupPackage } from "../../setupPackages";

export function useSetupPackages() {
  const [packages, setPackages] = useState<SetupPackage[]>([]);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    let active = true;
    let revision = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => {
      clearTimeout(retry);
      const request = ++revision;
      void listSetupPackages().then(
        (items) => {
          if (active && request === revision) {
            setPackages(items);
            setLoaded(true);
          }
        },
        (failure: unknown) => {
          if (!active || request !== revision) return;
          if (
            failure instanceof CommandFailure &&
            (failure.detail.code === "busy" || failure.detail.retryable)
          ) {
            // Initialization and native setup operations hold the connection guard.
            // Keep defaults pending and retain one bounded retry until it is released.
            retry = setTimeout(refresh, 1000);
          } else {
            setLoaded(true);
          }
        },
      );
    };
    refresh();
    window.addEventListener(SETUP_CHANGED_EVENT, refresh);
    return () => {
      active = false;
      clearTimeout(retry);
      window.removeEventListener(SETUP_CHANGED_EVENT, refresh);
    };
  }, []);
  return { packages, loaded };
}
