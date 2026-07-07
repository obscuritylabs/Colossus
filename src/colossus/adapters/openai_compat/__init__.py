"""OpenAI-compatible provider adapters."""

from colossus.adapters.openai_compat.chat import LocalOpenAIChatProvider
from colossus.adapters.openai_compat.responses import OpenAIResponsesProvider

__all__ = ["LocalOpenAIChatProvider", "OpenAIResponsesProvider"]
