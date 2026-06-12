"""Provider-safe tool name encoding."""

import re
from dataclasses import dataclass

from colossus.domain.errors import ProviderError
from colossus.domain.tools import ToolSpec

_UNSAFE_TOOL_NAME_CHARACTER = re.compile(r"[^A-Za-z0-9_-]")


@dataclass(frozen=True)
class ToolNameCodec:
    canonical_to_provider: dict[str, str]
    provider_to_canonical: dict[str, str]

    @classmethod
    def from_tools(cls, tools: tuple[ToolSpec, ...]) -> "ToolNameCodec":
        canonical_to_provider: dict[str, str] = {}
        provider_to_canonical: dict[str, str] = {}
        for tool in tools:
            provider_name = _encode_provider_tool_name(tool.name)
            existing = provider_to_canonical.get(provider_name)
            if existing is not None and existing != tool.name:
                raise ProviderError(
                    "Tool names collide after provider-safe encoding: "
                    f"{existing!r} and {tool.name!r} both map to {provider_name!r}."
                )
            canonical_to_provider[tool.name] = provider_name
            provider_to_canonical[provider_name] = tool.name
        return cls(
            canonical_to_provider=canonical_to_provider,
            provider_to_canonical=provider_to_canonical,
        )

    def encode(self, canonical_name: str) -> str:
        return self.canonical_to_provider.get(
            canonical_name,
            _encode_provider_tool_name(canonical_name),
        )

    def decode(self, provider_name: str) -> str:
        return self.provider_to_canonical.get(provider_name, provider_name)


def _encode_provider_tool_name(name: str) -> str:
    encoded = _UNSAFE_TOOL_NAME_CHARACTER.sub("_", name)
    if encoded:
        return encoded
    raise ProviderError("Tool name cannot be encoded for provider use: empty name.")
