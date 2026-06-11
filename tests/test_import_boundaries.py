import ast
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src" / "colossus"


def _imports(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    names: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module is not None:
            names.add(node.module)
    return names


def test_domain_does_not_import_outer_layers() -> None:
    for path in (SRC / "domain").glob("*.py"):
        forbidden = {
            name
            for name in _imports(path)
            if name.startswith("colossus.") and not name.startswith("colossus.domain")
        }
        assert forbidden == set(), f"{path} imports {forbidden}"


def test_application_does_not_import_adapters_or_interfaces() -> None:
    for path in (SRC / "application").glob("*.py"):
        forbidden = {
            name
            for name in _imports(path)
            if name.startswith(("colossus.adapters", "colossus.interfaces"))
        }
        assert forbidden == set(), f"{path} imports {forbidden}"
