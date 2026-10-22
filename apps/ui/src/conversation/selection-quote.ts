export const MAX_CONVERSATION_QUOTE_BYTES = 4096;

/** Keep complete UTF-8 characters inside the released quote budget. */
export function boundedConversationQuote(value: string): string {
  const bytes = new TextEncoder().encode(value.trim());
  // Streaming decode leaves an incomplete final character buffered rather than
  // replacing it with a larger replacement character outside the byte budget.
  return new TextDecoder()
    .decode(bytes.subarray(0, MAX_CONVERSATION_QUOTE_BYTES), { stream: true })
    .trim();
}
