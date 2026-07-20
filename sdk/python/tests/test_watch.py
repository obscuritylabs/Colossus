from __future__ import annotations

import math
import unittest
from collections.abc import AsyncIterator
from dataclasses import dataclass

from colossus_sdk.client import is_terminal_run_update
from colossus_sdk.watch import (
    RunFeedItem,
    RunFeedProtocolError,
    RunWatchReconciliation,
    watch_run,
)


@dataclass(frozen=True)
class Value:
    terminal: bool


async def no_sleep(_seconds: float) -> None:
    return None


async def reconcile_terminal(run_id: str, last_sequence: int) -> RunWatchReconciliation:
    return RunWatchReconciliation(run_id, last_sequence, True)


class TransientError(RuntimeError):
    def code(self) -> int:
        return 14


class FakeRunUpdate:
    def __init__(self, update_case: str | None) -> None:
        self.update_case = update_case

    def WhichOneof(self, name: str) -> str | None:
        if name != "update":
            raise AssertionError(f"unexpected oneof name: {name}")
        return self.update_case


class WatchTests(unittest.IsolatedAsyncioTestCase):
    def test_only_exact_terminal_update_variants_stop_watch(self) -> None:
        for update_case in ("result", "failure", "cancellation"):
            with self.subTest(update_case=update_case):
                self.assertTrue(is_terminal_run_update(FakeRunUpdate(update_case)))
        for update_case in (None, "state", "notice", "message"):
            with self.subTest(update_case=update_case):
                self.assertFalse(is_terminal_run_update(FakeRunUpdate(update_case)))

    async def test_reconnects_from_cursor_and_removes_duplicate(self) -> None:
        attempts = 0
        opened_after: list[int] = []

        async def open_watch(
            run_id: str,
            after_sequence: int,
        ) -> AsyncIterator[RunFeedItem[Value]]:
            nonlocal attempts
            attempts += 1
            opened_after.append(after_sequence)
            if attempts == 1:
                yield RunFeedItem(run_id, 1, Value(False))
                raise TransientError("transient")
            yield RunFeedItem(run_id, 1, Value(False))
            yield RunFeedItem(run_id, 2, Value(True))

        observed = [
            item.sequence
            async for item in watch_run(
                "run-1",
                open_watch,
                lambda value: value.terminal,
                reconcile_terminal,
                sleep=no_sleep,
            )
        ]

        self.assertEqual(opened_after, [0, 1])
        self.assertEqual(observed, [1, 2])

    async def test_clean_eof_at_terminal_cursor_does_not_reconnect(self) -> None:
        attempts = 0

        async def open_watch(
            _run_id: str,
            after_sequence: int,
        ) -> AsyncIterator[RunFeedItem[Value]]:
            nonlocal attempts
            attempts += 1
            self.assertEqual(after_sequence, 9)
            if False:
                yield RunFeedItem("", 0, Value(False))

        observed = [
            item.sequence
            async for item in watch_run(
                "run-1",
                open_watch,
                lambda value: value.terminal,
                reconcile_terminal,
                after_sequence=9,
                sleep=no_sleep,
            )
        ]
        self.assertEqual(attempts, 1)
        self.assertEqual(observed, [])

    async def test_fails_closed_on_gap(self) -> None:
        async def open_watch(
            run_id: str,
            _after_sequence: int,
        ) -> AsyncIterator[RunFeedItem[Value]]:
            yield RunFeedItem(run_id, 2, Value(True))

        with self.assertRaises(RunFeedProtocolError):
            async for _item in watch_run(
                "run-1",
                open_watch,
                lambda value: value.terminal,
                reconcile_terminal,
            ):
                pass

    async def test_rejects_zero_or_nonfinite_backoff(self) -> None:
        async def open_watch(
            _run_id: str,
            _after_sequence: int,
        ) -> AsyncIterator[RunFeedItem[Value]]:
            if False:
                yield RunFeedItem("", 0, Value(False))

        for initial_backoff, maximum_backoff in (
            (0.0, 1.0),
            (math.nan, 1.0),
            (0.25, math.inf),
        ):
            with (
                self.subTest(
                    initial_backoff=initial_backoff,
                    maximum_backoff=maximum_backoff,
                ),
                self.assertRaises(ValueError),
            ):
                async for _item in watch_run(
                    "run-1",
                    open_watch,
                    lambda value: value.terminal,
                    reconcile_terminal,
                    initial_backoff=initial_backoff,
                    maximum_backoff=maximum_backoff,
                ):
                    pass

    async def test_retries_only_retryable_errors(self) -> None:
        attempts = 0

        async def open_watch(
            run_id: str,
            _after_sequence: int,
        ) -> AsyncIterator[RunFeedItem[Value]]:
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                raise TransientError("transient")
            yield RunFeedItem(run_id, 1, Value(True))

        observed = [
            item.sequence
            async for item in watch_run(
                "run-1",
                open_watch,
                lambda value: value.terminal,
                reconcile_terminal,
                sleep=no_sleep,
            )
        ]
        self.assertEqual(attempts, 2)
        self.assertEqual(observed, [1])

    async def test_clean_eof_requires_exact_terminal_reconciliation(self) -> None:
        async def open_watch(
            _run_id: str,
            _after_sequence: int,
        ) -> AsyncIterator[RunFeedItem[Value]]:
            if False:
                yield RunFeedItem("", 0, Value(False))

        async def mismatched(
            run_id: str,
            last_sequence: int,
        ) -> RunWatchReconciliation:
            return RunWatchReconciliation(run_id, last_sequence + 1, True)

        with self.assertRaisesRegex(RunFeedProtocolError, "exact cursor"):
            async for _item in watch_run(
                "run-1",
                open_watch,
                lambda value: value.terminal,
                mismatched,
                after_sequence=9,
            ):
                pass


if __name__ == "__main__":
    unittest.main()
