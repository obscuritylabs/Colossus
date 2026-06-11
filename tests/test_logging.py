import logging

from colossus.infrastructure.logging import configure_logging


def test_configure_logging_suppresses_http_transport_logs_by_default() -> None:
    configure_logging(verbose=False)

    assert logging.getLogger().getEffectiveLevel() == logging.WARNING
    assert logging.getLogger("httpx").getEffectiveLevel() == logging.WARNING
    assert logging.getLogger("httpcore").getEffectiveLevel() == logging.WARNING


def test_configure_logging_enables_transport_logs_when_verbose() -> None:
    configure_logging(verbose=True)

    assert logging.getLogger().getEffectiveLevel() == logging.DEBUG
    assert logging.getLogger("httpx").getEffectiveLevel() == logging.DEBUG
    assert logging.getLogger("httpcore").getEffectiveLevel() == logging.DEBUG
