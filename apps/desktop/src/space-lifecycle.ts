import type {
  DesktopStatus,
  RuntimeTarget,
  SpaceSummary,
  WorkspaceSummary,
} from "./types";

function workspaceForSpace(space: SpaceSummary): WorkspaceSummary {
  return {
    workspaceId: space.spaceId,
    displayName: space.displayName,
    displayPath: space.displayPath,
  };
}

function managedTargetForSpace(
  status: DesktopStatus,
  space: SpaceSummary,
): RuntimeTarget {
  const existing = status.targets.find(
    (target) => target.targetId === space.targetId,
  );
  return {
    targetId: space.targetId,
    kind: "managed_local",
    label: space.displayName,
    state: space.state,
    message: space.message,
    selected: space.selected,
    terminalAvailable: space.state === "ready",
    workspace: existing?.workspace ?? workspaceForSpace(space),
    failureCode: existing?.failureCode ?? null,
  };
}

function projectManagedTargets(
  status: DesktopStatus,
  spaces: SpaceSummary[],
): RuntimeTarget[] {
  return [
    ...spaces
      .filter((space) => !space.archived)
      .map((space) => managedTargetForSpace(status, space)),
    ...status.targets.filter((target) => target.kind === "external_daemon"),
  ];
}

export function projectSpaceArchived(
  status: DesktopStatus,
  spaceId: string,
): DesktopStatus {
  const space = status.spaces.find(
    (candidate) => candidate.spaceId === spaceId,
  );
  if (space === undefined || space.archived) {
    return status;
  }
  const selected = status.selectedSpaceId === spaceId;
  const nextSpace = selected
    ? status.spaces
        .filter(
          (candidate) => candidate.spaceId !== spaceId && !candidate.archived,
        )
        .reduce<SpaceSummary | null>(
          (latest, candidate) =>
            latest === null || candidate.lastOpenedAtMs > latest.lastOpenedAtMs
              ? candidate
              : latest,
          null,
        )
    : null;
  const spaces = status.spaces.map((candidate) => {
    if (candidate.spaceId === spaceId) {
      return {
        ...candidate,
        archived: true,
        selected: false,
        state: "archived" as const,
      };
    }
    if (selected) {
      const isNext = candidate.spaceId === nextSpace?.spaceId;
      return {
        ...candidate,
        selected: isNext,
        state: isNext ? ("ready" as const) : candidate.state,
        message: isNext ? "Fixture runtime ready." : candidate.message,
      };
    }
    return candidate;
  });
  return {
    ...status,
    connection: selected
      ? nextSpace === null
        ? {
            state: "disconnected",
            message: `${space.displayName} was archived. Add a Workspace to continue.`,
            targetId: null,
          }
        : {
            state: "connected",
            message: "Fixture runtime ready.",
            targetId: nextSpace.targetId,
          }
      : status.connection,
    targets: projectManagedTargets(status, spaces),
    selectedTargetId: selected
      ? (nextSpace?.targetId ?? null)
      : status.selectedTargetId,
    selectedSpaceId: selected
      ? (nextSpace?.spaceId ?? null)
      : status.selectedSpaceId,
    managedState: selected
      ? nextSpace === null
        ? "needs_workspace"
        : "ready"
      : status.managedState,
    workspace: selected
      ? nextSpace === null
        ? null
        : workspaceForSpace(nextSpace)
      : status.workspace,
    spaces,
  };
}

export function projectSpaceRestored(
  status: DesktopStatus,
  spaceId: string,
): DesktopStatus {
  if (
    !status.spaces.some((space) => space.spaceId === spaceId && space.archived)
  ) {
    return status;
  }
  const spaces = status.spaces.map((space) =>
    space.spaceId === spaceId
      ? {
          ...space,
          archived: false,
          selected: false,
          state: "sleeping" as const,
          message: "Starts when selected.",
        }
      : space,
  );
  return { ...status, spaces, targets: projectManagedTargets(status, spaces) };
}
