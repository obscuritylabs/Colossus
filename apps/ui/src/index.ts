/// <reference path="./assets.d.ts" />
export { SettingsFrame } from "./components/SettingsFrame.js";
export { GraphWorkspace } from "./components/GraphWorkspace.js";
export {
  ComposerInput,
  ComposerModeSwitch,
  ComposerSendButton,
  isComposerSendKey,
} from "./components/Composer.js";
export { useComposerAutosize } from "./hooks/useComposerAutosize.js";
export {
  DropdownSelect,
  dropdownOptions,
  nextDropdownOptionIndex,
} from "./components/DropdownSelect.js";
export type {
  DropdownSelectChangeEvent,
  DropdownSelectOption,
} from "./components/DropdownSelect.js";

export { Button, TextInput } from "./components/Controls.js";
export {
  WorkspaceSidebarHeading,
  WorkspaceSidebarSearch,
  WorkspaceSidebarScope,
  WorkspaceSidebarWorkspace,
  WorkspaceSidebarGroupHeading,
  WorkspaceSidebarThreadContent,
} from "./components/WorkspaceSidebar.js";
export type { WorkspaceSidebarScopeValue } from "./components/WorkspaceSidebar.js";
export { RadioGroup } from "./components/RadioGroup.js";
export type { RadioGroupOption } from "./components/RadioGroup.js";
export { ApplicationFrame } from "./components/ApplicationFrame.js";
export { CatalogInventory } from "./components/CatalogInventory.js";
export type {
  CatalogInventoryRow,
  CatalogInventoryKind,
} from "./components/CatalogInventory.js";

export { ControlPlaneFrame } from "./components/ControlPlaneFrame.js";
export {
  WorkWelcome,
  DEFAULT_WORK_STARTERS,
} from "./components/WorkWelcome.js";
export type { WorkWelcomeProps } from "./components/WorkWelcome.js";
export type {
  ControlPlaneNavigation,
  ClassificationBanner,
} from "./components/ControlPlaneFrame.js";
