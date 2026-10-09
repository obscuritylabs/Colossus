import type { ComponentProps } from "react";
import * as Shared from "@colossus/ui/session/SessionWorkspace";
import { SessionPresentationProvider } from "@colossus/ui/session/links";
import { BrowserLink } from "./browser/BrowserLink";
export { SessionWorkspaceTabs } from "@colossus/ui/conversation";
export type { SessionWorkspaceView } from "@colossus/ui/conversation";
export function SessionTopology(
  props: ComponentProps<typeof Shared.SessionTopology>,
) {
  return (
    <SessionPresentationProvider linkComponent={BrowserLink}>
      <Shared.SessionTopology {...props} />
    </SessionPresentationProvider>
  );
}
export function SessionPlansView(
  props: ComponentProps<typeof Shared.SessionPlansView>,
) {
  return (
    <SessionPresentationProvider linkComponent={BrowserLink}>
      <Shared.SessionPlansView {...props} />
    </SessionPresentationProvider>
  );
}
export function SessionSourcesView(
  props: ComponentProps<typeof Shared.SessionSourcesView>,
) {
  return (
    <SessionPresentationProvider linkComponent={BrowserLink}>
      <Shared.SessionSourcesView {...props} />
    </SessionPresentationProvider>
  );
}
export function SessionSnapshotsView(
  props: ComponentProps<typeof Shared.SessionSnapshotsView>,
) {
  return (
    <SessionPresentationProvider linkComponent={BrowserLink}>
      <Shared.SessionSnapshotsView {...props} />
    </SessionPresentationProvider>
  );
}
export function SessionResourcesView(
  props: ComponentProps<typeof Shared.SessionResourcesView>,
) {
  return (
    <SessionPresentationProvider linkComponent={BrowserLink}>
      <Shared.SessionResourcesView {...props} />
    </SessionPresentationProvider>
  );
}
