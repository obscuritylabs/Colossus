"""Platform-specific runtime paths."""

from pathlib import Path

from platformdirs import user_config_dir, user_data_dir

APP_NAME = "colossus"
APP_AUTHOR = "colossus"


def config_dir() -> Path:
    return Path(user_config_dir(APP_NAME, APP_AUTHOR))


def data_dir() -> Path:
    return Path(user_data_dir(APP_NAME, APP_AUTHOR))


def config_path() -> Path:
    return config_dir() / "config.json"
