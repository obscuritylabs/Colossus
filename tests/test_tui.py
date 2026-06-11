import pytest

from colossus.interfaces.tui import ColossusTui


@pytest.mark.asyncio
async def test_tui_mounts() -> None:
    app = ColossusTui()

    async with app.run_test():
        assert app.query_one("#conversation-log") is not None
        assert app.query_one("#timeline-log") is not None
        assert app.query_one("#context-log") is not None
