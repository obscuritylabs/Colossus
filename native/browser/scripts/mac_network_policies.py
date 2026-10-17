"""Embed only the byte-exact, offline, locked Chromium macOS policies."""
import argparse
import hashlib
import json
from pathlib import Path


def generate(root: Path) -> str:
    lock = json.loads((root / "policies.lock.json").read_text())
    if lock["schema_version"] != 1 or lock["chromium_version"] != "154.0.8037.98":
        raise ValueError("unrecognized macOS policy version")
    expected = {"common", "renderer", "network", "gpu", "utility", "audio", "cdm",
                "mirroring", "on_device_model_execution", "on_device_translation",
                "print_backend", "print_compositor", "proxy_resolver", "screen_ai",
                "speech_recognition", "webnn_model_compilation"}
    if set(lock["files"]) != {name + ".sb" for name in expected}:
        raise ValueError("unrecognized macOS policy inventory")
    result = ["/* Generated from exact Chromium 154 policy bytes; see sandbox/LICENSE. */",
              "#ifndef COLOSSUS_MAC_NETWORK_POLICIES_GENERATED_H_",
              "#define COLOSSUS_MAC_NETWORK_POLICIES_GENERATED_H_"]
    for name, record in sorted(lock["files"].items()):
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise ValueError("policy is not a regular file")
        data = path.read_bytes()
        if len(data) > 64 * 1024:
            raise ValueError("policy oversized")
        blob = b"blob " + str(len(data)).encode() + b"\0" + data
        if (hashlib.sha256(data).hexdigest() != record["sha256"] or
                hashlib.sha1(blob).hexdigest() != record["git_blob_sha"]):
            raise ValueError("macOS policy differs from pinned source")
        key = name.removesuffix(".sb")
        result.append(f"static const char colossus_policy_{key}[] =")
        result.extend(json.dumps(line, ensure_ascii=False)
                      for line in data.decode("utf-8").splitlines(keepends=True))
        result.append(";")
        result.append(f'static const char colossus_policy_{key}_sha256[] = "{record["sha256"]}";')
    result.extend(["#endif", ""])
    return "\n".join(result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    options = parser.parse_args()
    options.output.write_text(generate(options.source))
