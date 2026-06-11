import json

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink


@pytest.mark.asyncio
async def test_audit_records_are_hash_chained(tmp_path) -> None:
    path = tmp_path / "audit.jsonl"
    sink = JsonlAuditSink(path)

    first = await sink.record("agent", "run.started", {"run_id": "one"})
    second = await sink.record("agent", "run.completed", {"run_id": "one"})

    lines = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    assert lines[0]["hash"] == first.hash
    assert second.prev_hash == first.hash
    assert lines[1]["prev_hash"] == first.hash
