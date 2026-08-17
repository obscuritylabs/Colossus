export const CONVERSATION_FOLLOW_THRESHOLD = 96;

export interface ScrollPosition {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

export function isNearConversationLatest(
  position: ScrollPosition,
  threshold = CONVERSATION_FOLLOW_THRESHOLD,
): boolean {
  return (
    position.scrollHeight - position.scrollTop - position.clientHeight <=
    threshold
  );
}
