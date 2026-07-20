"""Fail-closed generated-contract checks for Python package builds."""

from __future__ import annotations

import hashlib
import re
from pathlib import Path

ROOT = Path(__file__).parent
_DIGEST_PATTERN = re.compile(r"[0-9a-f]{64}")
_EXPLICIT_SOURCE_INPUTS = (
    "sdk/buf.gen.yaml",
    "sdk/package.json",
    "sdk/package-lock.json",
    "sdk/python/requirements-codegen.txt",
    "sdk/scripts/generated-input-digest",
    "sdk/scripts/generated-output-digest",
    "sdk/scripts/generate",
    "sdk/scripts/install-codegen-tools",
    "sdk/typescript/package.json",
    "sdk/typescript/package-lock.json",
)


def _read_digest(path: Path, description: str) -> str:
    try:
        lines = path.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeError) as error:
        raise RuntimeError(f"{description} is missing or unreadable") from error
    if len(lines) != 1 or _DIGEST_PATTERN.fullmatch(lines[0]) is None:
        raise RuntimeError(f"{description} is invalid; run sdk/scripts/generate")
    return lines[0]


def generated_digest(root: Path = ROOT) -> str:
    generated_root = root / "generated"
    generated_files = sorted(
        path
        for path in generated_root.rglob("*")
        if path.is_file() and (path.suffix in {".py", ".pyi"} or path.name == "py.typed")
    )
    if not generated_files:
        raise RuntimeError("generated Colossus API modules are missing")
    entries = bytearray()
    for path in generated_files:
        relative_path = path.relative_to(root).as_posix()
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        entries.extend(f"{digest}  {relative_path}\n".encode())
    return hashlib.sha256(entries).hexdigest()


def source_input_paths(repository_root: Path) -> tuple[Path, ...]:
    api_root = repository_root / "api"
    inputs = [
        path
        for path in api_root.rglob("*")
        if path.is_file() and (path.suffix == ".proto" or path.name == "buf.yaml")
    ]
    inputs.extend(repository_root / relative for relative in _EXPLICIT_SOURCE_INPUTS)
    return tuple(sorted(inputs, key=lambda path: path.relative_to(repository_root).as_posix()))


def source_input_digest(repository_root: Path) -> str:
    entries = bytearray()
    for path in source_input_paths(repository_root):
        if not path.is_file():
            relative_path = path.relative_to(repository_root).as_posix()
            raise RuntimeError(f"generated SDK input is missing: {relative_path}")
        relative_path = path.relative_to(repository_root).as_posix()
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        entries.extend(f"{digest}  {relative_path}\n".encode())
    return hashlib.sha256(entries).hexdigest()


def verify_source_inputs(root: Path = ROOT) -> None:
    sdk_root = root.parent
    repository_root = sdk_root.parent
    input_manifest = sdk_root / "generated-inputs.sha256"
    source_markers = (
        repository_root / "api/buf.yaml",
        sdk_root / "buf.gen.yaml",
        sdk_root / "scripts/generated-input-digest",
        input_manifest,
    )
    present = tuple(path.is_file() for path in source_markers)
    if not any(present):
        # A source distribution carries generated output and its output manifest,
        # but intentionally does not carry the repository's schema/tool inputs.
        return
    if not all(present):
        raise RuntimeError(
            "generated SDK source input gate is incomplete; use a complete source checkout"
        )

    expected_digest = _read_digest(
        input_manifest,
        "generated SDK source input manifest",
    )
    if source_input_digest(repository_root) != expected_digest:
        raise RuntimeError(
            "generated Python bindings do not match the schema/tool inputs; "
            "run sdk/scripts/generate"
        )


def verify_generated(root: Path = ROOT) -> None:
    generated_root = root / "generated"
    generated_manifest = root / "generated-output.sha256"
    required_generated_files = (
        generated_root / "colossus/api/v1alpha1/agent_run_pb2.py",
        generated_root / "colossus/api/v1alpha1/agent_run_pb2_grpc.py",
        generated_root / "colossus/api/v1alpha1/py.typed",
        generated_manifest,
    )
    missing = [path for path in required_generated_files if not path.is_file()]
    if missing:
        raise RuntimeError("generated Colossus API modules are missing; run sdk/scripts/generate")
    if (generated_root / "google").exists():
        raise RuntimeError(
            "generated google modules conflict with canonical Google Protobuf packages; "
            "run sdk/scripts/generate"
        )

    expected_digest = _read_digest(
        generated_manifest,
        "generated SDK release manifest",
    )
    if generated_digest(root) != expected_digest:
        raise RuntimeError(
            "generated Colossus API modules do not match their release manifest; "
            "run sdk/scripts/generate"
        )
    verify_source_inputs(root)
