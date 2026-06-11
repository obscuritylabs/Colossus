"""Logging setup."""

import logging

import structlog


def configure_logging(verbose: bool = False) -> None:
    level = logging.DEBUG if verbose else logging.WARNING
    logging.basicConfig(level=level, force=True)
    for logger_name in ("httpx", "httpcore"):
        logging.getLogger(logger_name).setLevel(level)
    structlog.configure(
        wrapper_class=structlog.make_filtering_bound_logger(
            level
        ),
        processors=[
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.JSONRenderer(),
        ],
    )
