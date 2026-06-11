"""Helpers for normalizing provider model catalog responses."""

from colossus.domain.providers import ProviderModelInfo


def extract_model_infos(data: object) -> tuple[ProviderModelInfo, ...]:
    if not isinstance(data, dict):
        return ()
    entries = data.get("data")
    if not isinstance(entries, list):
        return ()
    models: list[ProviderModelInfo] = []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        model_id = entry.get("id")
        if not isinstance(model_id, str):
            continue
        owner = entry.get("owned_by")
        created = entry.get("created")
        models.append(
            ProviderModelInfo(
                id=model_id,
                owner=owner if isinstance(owner, str) else None,
                created=created if isinstance(created, int) else None,
                context_window_tokens=_context_window_tokens(entry),
                max_output_tokens=_max_output_tokens(entry),
            )
        )
    return tuple(models)


def _context_window_tokens(entry: dict[str, object]) -> int | None:
    return _first_positive_int(
        entry.get("context_length"),
        entry.get("context_window"),
        entry.get("max_context_length"),
        entry.get("max_model_len"),
        _nested_value(entry, "top_provider", "context_length"),
    )


def _max_output_tokens(entry: dict[str, object]) -> int | None:
    return _first_positive_int(
        entry.get("max_completion_tokens"),
        entry.get("max_output_tokens"),
        _nested_value(entry, "top_provider", "max_completion_tokens"),
        _nested_value(entry, "top_provider", "max_output_tokens"),
    )


def _nested_value(entry: dict[str, object], parent: str, child: str) -> object:
    value = entry.get(parent)
    if isinstance(value, dict):
        return value.get(child)
    return None


def _first_positive_int(*values: object) -> int | None:
    for value in values:
        parsed = _positive_int(value)
        if parsed is not None:
            return parsed
    return None


def _positive_int(value: object) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value if value > 0 else None
    if isinstance(value, float) and value.is_integer():
        parsed = int(value)
        return parsed if parsed > 0 else None
    if isinstance(value, str) and value.isdigit():
        parsed = int(value)
        return parsed if parsed > 0 else None
    return None
