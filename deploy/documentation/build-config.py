"""Select portable documentation URLs without changing the GitHub Pages source."""

from pathlib import Path
import os
import re
import sys
import tomllib


def normalize_timestamps(root: Path) -> None:
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    if epoch < 0 or epoch > 253402300799:
        raise ValueError("SOURCE_DATE_EPOCH is outside the supported UTC range")
    if epoch:
        for path in [root, *root.rglob("*")]:
            if not path.is_symlink():
                os.utime(path, (epoch, epoch))


def configure(source: Path, target: Path) -> None:
    if source.resolve() == target.resolve():
        raise ValueError("the canonical documentation source must not be overwritten")
    text = source.read_text(encoding="utf-8")
    project = tomllib.loads(text)["project"]
    if project["docs_dir"] != "docs" or project["site_dir"] != "site":
        raise ValueError("documentation paths must retain the canonical build contract")
    text, replaced = re.subn(
        r'^site_url\s*=\s*"[^"\r\n]*"\s*$',
        'site_url = "/docs/"',
        text,
        count=1,
        flags=re.MULTILINE,
    )
    if replaced != 1 or tomllib.loads(text)["project"]["site_url"] != "/docs/":
        raise ValueError("canonical documentation site URL could not be selected")
    # Zensical 0.0.50's instant-navigation sitemap reader requires absolute URLs.
    # Portable root-relative pages retain ordinary navigation and local search.
    text = re.sub(
        r'^[ \t]*"navigation\.instant(?:\.progress)?",[ \t]*\r?\n',
        "",
        text,
        flags=re.MULTILINE,
    )
    features = tomllib.loads(text)["project"]["theme"]["features"]
    if any(
        name in features
        for name in ("navigation.instant", "navigation.instant.progress")
    ):
        raise ValueError("portable documentation must use full-page navigation")
    target.write_text(text, encoding="utf-8")
    normalize_timestamps(target.parent / "docs")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: build-config.py SOURCE.toml TARGET.toml")
    if sys.argv[1] == "--timestamps":
        normalize_timestamps(Path(sys.argv[2]))
    else:
        configure(Path(sys.argv[1]), Path(sys.argv[2]))
