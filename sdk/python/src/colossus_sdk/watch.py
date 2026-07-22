"""Durable run-feed replay and reconnect support."""

from __future__ import annotations

import asyncio
import math
from collections.abc import AsyncIterator, Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

Value = TypeVar("Value")


@dataclass(frozen=True, slots=True)
class RunFeedItem(Generic[Value]):
    """One released durable feed item and its replay identity."""

    run_id: str
    sequence: int
    value: Value


class RunFeedProtocolError(RuntimeError):
    """The server stream violated cursor or run identity invariants."""


@dataclass(frozen=True, slots=True)
class RunWatchReconciliation:
    """GetRun evidence used to prove a clean watch close is final."""

    run_id: str
    last_sequence: int
    terminal: bool


OpenRunWatch = Callable[[str, int], AsyncIterator[RunFeedItem[Value]]]
IsTerminal = Callable[[Value], bool]
ReconcileRun = Callable[[str, int], Awaitable[RunWatchReconciliation]]
IsRetryable = Callable[[BaseException], bool]
Sleep = Callable[[float], Awaitable[None]]


def _default_retryable(error: BaseException) -> bool:
    code_method = getattr(error, "code", None)
    code = code_method() if callable(code_method) else code_method
    name = getattr(code, "name", None)
    return name == "UNAVAILABLE" or code == 14


async def watch_run(
    run_id: str,
    open_watch: OpenRunWatch[Value],
    is_terminal: IsTerminal[Value],
    reconcile: ReconcileRun,
    *,
    after_sequence: int = 0,
    is_retryable: IsRetryable = _default_retryable,
    initial_backoff: float = 0.25,
    maximum_backoff: float = 5.0,
    sleep: Sleep = asyncio.sleep,
) -> AsyncIterator[RunFeedItem[Value]]:
    """Replay and tail one run, reconnecting only this read-only operation."""

    if not run_id:
        raise ValueError("run_id must not be empty")
    if after_sequence < 0:
        raise ValueError("after_sequence must be non-negative")
    if (
        not math.isfinite(initial_backoff)
        or not math.isfinite(maximum_backoff)
        or initial_backoff <= 0
        or maximum_backoff < initial_backoff
    ):
        raise ValueError("watch backoff bounds are invalid")

    cursor = after_sequence
    backoff = initial_backoff
    while True:
        try:
            async for item in open_watch(run_id, cursor):
                if item.run_id != run_id:
                    raise RunFeedProtocolError("watch stream returned a different run_id")
                if item.sequence <= cursor:
                    continue
                if item.sequence != cursor + 1:
                    raise RunFeedProtocolError("watch stream contains a sequence gap")

                cursor = item.sequence
                backoff = initial_backoff
                yield item
                if is_terminal(item.value):
                    return

            reconciled = await reconcile(run_id, cursor)
            if (
                reconciled.run_id != run_id
                or reconciled.last_sequence != cursor
                or not reconciled.terminal
            ):
                raise RunFeedProtocolError("clean watch EOF was not terminal at the exact cursor")
            return
        except RunFeedProtocolError:
            raise
        except asyncio.CancelledError:
            raise
        except BaseException as error:
            if not is_retryable(error):
                raise
            await sleep(backoff)
            backoff = min(maximum_backoff, max(0.001, backoff * 2))
