import { createContext, useContext, useEffect, useState } from "react";
import { getManagedConfiguration } from "./api";
import { SETUP_CHANGED_EVENT } from "./setupPackages";
import type { ProviderPresentation } from "./types";
import type { ReactNode } from "react";
import { useSetupPackages } from "./components/setup/useSetupPackages";
import type { ProviderConnectionIdentity } from "./providerBrand";

type Presentation = ProviderPresentation & ProviderConnectionIdentity;
const Context = createContext<Presentation[]>([]);
export function SetupPresentationProvider({
  children,
}: {
  children: ReactNode;
}) {
  const { packages } = useSetupPackages();
  const [saved, setSaved] = useState<Presentation[]>([]);
  useEffect(() => {
    let generation = 0;
    async function refresh() {
      const request = ++generation;
      try {
        const snapshot = await getManagedConfiguration();
        if (request !== generation) return;
        setSaved(
          snapshot.globalConfiguration.providers
            .filter((entry) => !entry.archived)
            .flatMap((entry) => {
              const provider = entry.revisions.find(
                (revision) => revision.revision === entry.currentRevision,
              )?.value;
              const presentation = snapshot.providerPresentations?.[entry.id];
              return provider && presentation
                ? [{ ...provider, ...presentation }]
                : [];
            }),
        );
      } catch {
        /* Setup fixtures and unavailable runtimes retain package presentation. */
      }
    }
    void refresh();
    window.addEventListener(SETUP_CHANGED_EVENT, refresh);
    return () => {
      generation++;
      window.removeEventListener(SETUP_CHANGED_EVENT, refresh);
    };
  }, []);
  const providers = [...saved, ...packages.flatMap((entry) => entry.providers)];
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
