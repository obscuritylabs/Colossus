"""Display helpers for elapsed run timings."""

from __future__ import annotations


def format_elapsed(seconds: float) -> str:
    normalized = max(seconds, 0.0)
    if normalized < 1:
        milliseconds = round(normalized * 1000)
        if milliseconds == 0 and normalized > 0:
            return "<1ms"
        return f"{milliseconds}ms"
    if normalized < 60:
        return f"{normalized:.1f}s"
    minutes, remaining_seconds = divmod(normalized, 60)
    if minutes < 60:
        return f"{int(minutes)}m {round(remaining_seconds)}s"
    hours, remaining_minutes = divmod(minutes, 60)
    return f"{int(hours)}h {int(remaining_minutes)}m"
