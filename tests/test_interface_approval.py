from rich.console import Console

from colossus.domain.policy import PolicyDecision
from colossus.domain.tools import ToolCall
from colossus.interfaces import approval as approval_module
from colossus.interfaces.approval import RichApprovalHandler


async def test_rich_approval_handler_renders_and_returns_decision(monkeypatch) -> None:
    console = Console(record=True, width=120)
    captured: dict[str, object] = {}

    def fake_confirm_ask(*args, **kwargs) -> bool:
        captured["prompt"] = args[0]
        captured["default"] = kwargs["default"]
        return True

    monkeypatch.setattr(approval_module.Confirm, "ask", fake_confirm_ask)
    handler = RichApprovalHandler(console)

    approved = await handler.approve(
        ToolCall(
            call_id="call-1",
            name="filesystem.write",
            arguments={"path": "note.txt", "content": "hello"},
        ),
        PolicyDecision(decision="requires_approval", reason="Tool requires approval."),
    )

    output = console.export_text()
    assert approved is True
    assert captured == {"prompt": "Approve this tool call?", "default": False}
    assert "approval required filesystem.write" in output
    assert '"path": "note.txt"' in output
