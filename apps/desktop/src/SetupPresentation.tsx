import { createContext, useContext, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { listSetupPackages } from "./api";
import { SETUP_CHANGED_EVENT } from "./setupPackages";
import type { SetupProvider } from "./setupPackages";
import type { ProviderConnectionIdentity } from "./providerBrand";

const Context = createContext<SetupProvider[]>([]);
export function SetupPresentationProvider({
  children,
}: {
  children: ReactNode;
}) {
  const [providers, setProviders] = useState<SetupProvider[]>([]);
  useEffect(() => {
    let active = true;
    let revision = 0;
    const refresh = () => {
      const request = ++revision;
      void listSetupPackages().then(
        (packages) => {
          if (active && request === revision)
            setProviders(packages.flatMap((p) => p.providers));
        },
        () => {},
      );
    };
    refresh();
    window.addEventListener(SETUP_CHANGED_EVENT, refresh);
    return () => {
      active = false;
      window.removeEventListener(SETUP_CHANGED_EVENT, refresh);
    };
  }, []);
  return <Context.Provider value={providers}>{children}</Context.Provider>;
}
export function useSetupPresentation(
  provider?: ProviderConnectionIdentity | null,
) {
  return useContext(Context).find(
    (p) =>
      provider &&
      p.kind === provider.kind &&
      p.baseUrl === provider.baseUrl &&
      (!provider.profile || p.profile === provider.profile),
  );
}
