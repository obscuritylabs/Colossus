import { createContext, useContext } from "react";
import type { ReactNode } from "react";
import { useSetupPackages } from "./components/setup/useSetupPackages";
import type { SetupProvider } from "./setupPackages";
import type { ProviderConnectionIdentity } from "./providerBrand";

const Context = createContext<SetupProvider[]>([]);
export function SetupPresentationProvider({
  children,
}: {
  children: ReactNode;
}) {
  const { packages } = useSetupPackages();
  const providers = packages.flatMap((entry) => entry.providers);
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
