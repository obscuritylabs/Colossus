"""Append-only hash-chained JSONL audit sink."""

import hashlib
import json
from pathlib import Path

from colossus.domain.audit import AuditRecord


class JsonlAuditSink:
    def __init__(self, path: Path) -> None:
        self._path = path
        self._path.parent.mkdir(parents=True, exist_ok=True)

    async def record(self, actor: str, event: str, details: dict[str, object]) -> AuditRecord:
        seq, prev_hash = self._last_state()
        record = AuditRecord(
            seq=seq + 1,
            prev_hash=prev_hash,
            actor=actor,
            event=event,
            details=details,
        )
        payload = record.model_dump(mode="json", exclude={"hash"})
        digest = hashlib.sha256(
            json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest()
        stored = record.model_copy(update={"hash": digest})
        with self._path.open("a", encoding="utf-8") as handle:
            handle.write(stored.model_dump_json() + "\n")
        return stored

    def _last_state(self) -> tuple[int, str]:
        if not self._path.exists():
            return 0, ""
        last_line = ""
        with self._path.open("r", encoding="utf-8") as handle:
            for line in handle:
                if line.strip():
                    last_line = line
        if not last_line:
            return 0, ""
        data = json.loads(last_line)
        return int(data["seq"]), str(data["hash"])
