#!/usr/bin/env python3
"""OFF-default native renderer NSS open-denial experiment; broker custody stays false."""
import argparse
import json
import os
from pathlib import Path
import sys
import tempfile

import component
from pki_fixture import Fixture
from pki_fixture_private import private_directory, private_write
from pki_fixture_proxy import FixtureProxy
from run_host_pki_probe import ProbeError, run_case, utility


def run(arguments):
    if sys.platform != "linux" or os.geteuid() == 0:
        raise ProbeError("native renderer experiment requires the non-root pinned Linux CEF lane")
    root = component.directory(arguments.component)
    inventory = component.verify_installed(root)
    if inventory["platform"] != "linux64":
        raise ProbeError("native renderer experiment requires the Linux component")
    host = (root / "colossus-native-browser-host").resolve(strict=True)
    host_digest = component.digest_file(host)
    manifest_digest = component.digest_file(root / component.MANIFEST)
    tools = {"certutil": utility(arguments.certutil), "pk12util": utility(arguments.pk12util)}
    evidence = arguments.evidence.absolute()
    private_directory(evidence)
    result = {"schema_version": 1, "native_renderer_direct_open_denied": False,
              "broker_private_key_custody_verified": False, "production_containment": False,
              "signed_installed_artifact_verified": False, "component": inventory,
              "host_sha256": host_digest, "component_manifest_sha256": manifest_digest,
              "nss_tool_sha256": {name: value["sha256"] for name, value in tools.items()},
              "case": None, "server_receipts": []}
    try:
        with tempfile.TemporaryDirectory(prefix="renderer-open-", dir=evidence) as allocation:
            allocation = Path(allocation)
            fixture = Fixture(allocation / "material", renderer_context=True)
            proxy = None
            try:
                fixture.start()
                proxy = FixtureProxy(fixture.metadata)
                case = {"name": "renderer_native_open", "endpoint": "mtls", "path": "/client-check",
                        "binding": "client", "identities": ["client"], "native_custody": True,
                        "title": "Colossus mTLS identity verified", "status": 200}
                result["case"] = run_case(case, fixture, proxy, root, host, tools, allocation, inventory)
                if component.verify_installed(root) != inventory \
                        or component.digest_file(host) != host_digest \
                        or component.digest_file(root / component.MANIFEST) != manifest_digest:
                    raise ProbeError("native renderer experiment artifact changed during acceptance")
                result["native_renderer_direct_open_denied"] = result["case"]["renderer_native_open_denied"]
                if not result["native_renderer_direct_open_denied"]:
                    raise ProbeError("native renderer NSS open-denial experiment unverified (receipt=" +
                                     result["case"]["native_receipt"] + ")")
            finally:
                result["server_receipts"] = fixture.report()
                if proxy is not None:
                    proxy.close()
                fixture.close()
        return result
    finally:
        private_write(evidence / "native-renderer-open-experiment.json",
                      (json.dumps(result, indent=2) + "\n").encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("component", "certutil", "pk12util", "evidence"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    try:
        print(json.dumps(run(parser.parse_args()), sort_keys=True))
        return 0
    except (ProbeError, component.ComponentError) as error:
        print(str(error), file=sys.stderr)
        return 1
    except OSError as error:
        print(f"native renderer experiment filesystem/process prerequisite failed (errno={error.errno})", file=sys.stderr)
        return 1
    except Exception as error:
        print(f"native renderer experiment failed ({type(error).__name__})", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
