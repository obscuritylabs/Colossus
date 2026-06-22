"""Helpers for provider-facing tool schemas."""

from copy import deepcopy
from typing import Any

INJECTED_ARGUMENT_MARKER = "x-colossus-injected"
PROVIDER_HIDDEN_ARGUMENT_MARKER = "x-colossus-provider-hidden"


def provider_input_schema(schema: dict[str, object]) -> dict[str, object]:
    """Return a provider-safe schema with Colossus-injected arguments hidden."""

    provider_schema = deepcopy(schema)
    properties = provider_schema.get("properties")
    if not isinstance(properties, dict):
        return provider_schema

    injected_names = {
        name
        for name, value in properties.items()
        if isinstance(name, str)
        and isinstance(value, dict)
        and (
            value.get(INJECTED_ARGUMENT_MARKER) is True
            or value.get(PROVIDER_HIDDEN_ARGUMENT_MARKER) is True
        )
    }
    if not injected_names:
        return provider_schema

    for name in injected_names:
        properties.pop(name, None)

    required = provider_schema.get("required")
    if isinstance(required, list):
        provider_schema["required"] = [
            name for name in required if not isinstance(name, str) or name not in injected_names
        ]

    return provider_schema


def injected_argument_schema(schema: dict[str, Any]) -> dict[str, Any]:
    return {**schema, INJECTED_ARGUMENT_MARKER: True}


def provider_hidden_argument_schema(schema: dict[str, Any]) -> dict[str, Any]:
    return {**schema, PROVIDER_HIDDEN_ARGUMENT_MARKER: True}
