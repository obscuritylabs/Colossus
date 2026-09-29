import { useEffect, useState } from "react";
import { listSetupPackages } from "../../api";
import { SETUP_CHANGED_EVENT, type SetupPackage } from "../../setupPackages";

export function useSetupPackages() {
  const [packages, setPackages] = useState<SetupPackage[]>([]);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    let active = true;
    let revision = 0;
    const refresh = () => {
      const request = ++revision;
      void listSetupPackages().then(
        (items) => {
          if (active && request === revision) {
            setPackages(items);
            setLoaded(true);
          }
        },
        () => {
          if (active && request === revision) setLoaded(true);
        },
      );
    };
    refresh();
    window.addEventListener(SETUP_CHANGED_EVENT, refresh);
    return () => {
      active = false;
      window.removeEventListener(SETUP_CHANGED_EVENT, refresh);
    };
  }, []);
  return { packages, loaded };
}
