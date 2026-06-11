"""Textual TUI surface."""

from typing import ClassVar

from textual.app import App, ComposeResult
from textual.binding import BindingType
from textual.containers import Horizontal, Vertical
from textual.widgets import Footer, Header, Input, Log, Static


class ColossusTui(App[None]):
    CSS = """
    Horizontal { height: 1fr; }
    #conversation { width: 2fr; }
    #side { width: 1fr; }
    Log { height: 1fr; border: solid $accent; }
    Static { padding: 0 1; }
    """

    BINDINGS: ClassVar[list[BindingType]] = [("q", "quit", "Quit")]

    def compose(self) -> ComposeResult:
        yield Header()
        with Horizontal():
            with Vertical(id="conversation"):
                yield Static("Conversation")
                yield Log(id="conversation-log")
                yield Input(placeholder="Ask Colossus...")
            with Vertical(id="side"):
                yield Static("Run Timeline")
                yield Log(id="timeline-log")
                yield Static("Context / Audit")
                yield Log(id="context-log")
                yield Static("Tool Calls / Approvals")
                yield Log(id="audit-log")
        yield Footer()

    def on_mount(self) -> None:
        self.query_one("#timeline-log", Log).write_line("TUI shell ready.")
        self.query_one("#context-log", Log).write_line(
            "Context auto-compaction is available in CLI and REPL."
        )
        self.query_one("#audit-log", Log).write_line("No run selected.")

    def on_input_submitted(self, event: Input.Submitted) -> None:
        self.query_one("#conversation-log", Log).write_line(f"> {event.value}")
        self.query_one("#timeline-log", Log).write_line("Use `colossus run` or REPL for execution.")
        self.query_one("#context-log", Log).write_line("No active TUI run context yet.")
        event.input.value = ""


def run_tui() -> None:
    ColossusTui().run()
