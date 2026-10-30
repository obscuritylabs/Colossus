/// <reference path="./assets.d.ts" />
export {
  ConversationEntry,
  ConversationTimeline,
} from "./components/Conversation.js";
export type { ConversationEntryProps } from "./components/Conversation.js";
export { MessageCopyButton } from "./components/MessageCopyButton.js";
export type { MessageCopyButtonProps } from "./components/MessageCopyButton.js";
export { ConversationUsageDetails } from "./components/ConversationUsage.js";
export { sumConversationUsage } from "./lib/conversation-usage.js";
export type { ConversationUsage } from "./lib/conversation-usage.js";
export { ConversationActivity } from "./components/ConversationActivity.js";
export type {
  ConversationActivityProps,
  ConversationActivityTone,
} from "./components/ConversationActivity.js";
export {
  ConversationComposer,
  ConversationComposerFrame,
} from "./components/ConversationComposer.js";
export {
  MarkdownContent,
  safeWebLink,
  markdownContentPropsAreEqual,
  MAX_MARKDOWN_CHARACTERS,
  MAX_MARKDOWN_AST_NODES,
} from "./components/MarkdownContent.js";
export type {
  MarkdownLink,
  MarkdownContentProps,
} from "./components/MarkdownContent.js";

export {
  WorkSurfaceHeader,
  SessionWorkspaceTabs,
  SESSION_WORKSPACE_VIEWS,
  WORK_STATUS_PRESENTATIONS,
} from "./components/WorkPresentation.js";
export type {
  SessionWorkspaceView,
  WorkStatusPresentation,
} from "./components/WorkPresentation.js";

export { ReleasedRunActivity } from "./components/ReleasedRunActivity.js";
export {
  ConversationSelectionLauncher,
  useConversationSelection,
} from "./conversation/ConversationSelection.js";
export type {
  ConversationSelection,
  ConversationSelectionBounds,
  ConversationSelectionLauncherProps,
  ConversationSelectionOptions,
  ConversationSelectionQuote,
} from "./conversation/ConversationSelection.js";
export {
  boundedConversationQuote,
  MAX_CONVERSATION_QUOTE_BYTES,
} from "./conversation/selection-quote.js";
export {
  NextPromptSuggestion,
  useNextPrompt,
} from "./components/NextPromptSuggestion.js";
export { localNextPrompt, normalizeNextPrompt } from "./lib/next-prompt.js";
export type {
  NextPromptMode,
  NextPromptSource,
  NextPromptPrediction,
} from "./lib/next-prompt.js";
