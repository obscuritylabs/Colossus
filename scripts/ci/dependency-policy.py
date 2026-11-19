#!/usr/bin/env python3
"""Adjudicate only the explicitly approved, expiring chrono support risk.

Scanner thresholds and findings are preserved. This is deliberately separate from
deny.toml ignores: current raw results, advisory bytes, and the complete reviewed
reverse dependency scope must agree before the one support warning is accepted.
Python 3.10 standard library only; cargo xtask owns invocation.
"""

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid


ADVISORY = "RUSTSEC-2026-0543"
SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
RSA = "RUSTSEC-2023-0071"
EXPIRY = "2026-12-19T00:00:00Z"
APPROVAL = {
    "reference": "call_7b8046c722cc4a63ac3c1dd3b7bf8f3b",
    "decision": "Approve 30-day scoped exception",
    "expires_utc": EXPIRY,
}
OBLIGATIONS = {
    "follow_up": "chrono-unmaintained-parent-graph-migration",
    "owner": "dependency_owner",
    "scope_owners": ["cloud_auth_owner", "mcp_owner", "desktop_owner"],
    "scope_due_utc": "2026-11-26T00:00:00Z",
    "implementation_review_owner": "maintenance_coordinator",
    "implementation_review_due_utc": "2026-12-12T00:00:00Z",
    "remove_or_block_before_utc": EXPIRY,
    "reevaluate_on_new_security_information": True,
    "automatic_renewal": False,
}
POLICY = "ci/dependency-policy/chrono-2026-0543.json"
GRAPHS = {"root": "Cargo.toml", "desktop": "apps/desktop/src-tauri/Cargo.toml"}
NOTICE = (
    "WARNING: accepted temporary support risk: chrono 0.4.45 / "
    f"{ADVISORY}; expires {EXPIRY}. Chrono remains unmaintained; "
    "this is not a chrono fix or a dependency-clean result."
)


class PolicyError(Exception):
    """Required evidence or the narrow approval boundary did not match."""


def require(condition, message):
    if not condition:
        raise PolicyError(message)


def normalized(value):
    """Normalize transport newlines, preserving all advisory meaning and fields."""
    if isinstance(value, str):
        return value.replace("\r\n", "\n")
    if isinstance(value, list):
        return [normalized(item) for item in value]
    if isinstance(value, dict):
        return {key: normalized(item) for key, item in value.items()}
    return value


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(normalized(value), sort_keys=True, separators=(",", ":")).encode()


def strict_json(data):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def invalid(value):
        raise PolicyError(f"invalid JSON constant: {value}")

    try:
        result = json.loads(data, object_pairs_hook=unique, parse_constant=invalid)
        require(isinstance(result, dict), "JSON object required")
        return result
    except (ValueError, UnicodeError) as error:
        raise PolicyError(f"malformed scanner/policy JSON: {error}") from error


def mapping(value, label):
    require(isinstance(value, dict), f"missing or malformed {label}")
    return value


def validate_policy(policy, advisory_bytes, now):
    mapping(policy, "policy record")
    require(policy.get("schema") == 1, "missing or unsupported policy schema")
    require(policy.get("approval") == APPROVAL, "missing or changed explicit approval")
    require(policy.get("removal") == OBLIGATIONS, "missing or changed removal owners/dates/conditions")
    require(isinstance(policy.get("rationale"), str) and policy["rationale"].strip(),
            "missing support-risk rationale")
    require(now.tzinfo is not None and now.utcoffset() == dt.timedelta(0), "UTC clock required")
    expiry = dt.datetime.fromisoformat(EXPIRY.replace("Z", "+00:00"))
    require(now < expiry, "chrono support-risk approval expired; no automatic renewal")
    require(policy.get("package") == {"name": "chrono", "version": "0.4.45", "source": SOURCE},
            "changed approved package/version/source")
    require(set(policy.get("graphs", {})) == set(GRAPHS), "changed approved graph scope")
    uses = mapping(policy.get("application_uses"), "direct application use bindings")
    require(set(uses) == set(GRAPHS), "changed direct application use graph scope")
    root_uses = mapping(uses["root"], "root application use bindings")
    mapping(uses["desktop"], "Desktop application use bindings")
    require(root_uses.get("files")
            and root_uses.get("references") == ["crates/colossus-cloud-server/src/auth.rs"],
            "missing or changed direct application use inventory")
    require(digest(advisory_bytes.replace(b"\r\n", b"\n")) == policy.get("advisory_sha256"),
            "current advisory content differs from reviewed fingerprint")
    finding = mapping(policy.get("reviewed_finding"), "reviewed finding")
    advisory = mapping(finding.get("advisory"), "reviewed advisory")
    require(advisory.get("id") == ADVISORY and advisory.get("package") == "chrono"
            and advisory.get("informational") == "unmaintained"
            and advisory.get("aliases") == [] and advisory.get("cvss") is None
            and advisory.get("withdrawn") is None
            and finding.get("affected") is None
            and finding.get("versions") == {"patched": [], "unaffected": []},
            "approval covers only the reviewed unmaintained advisory, without aliases or vulnerability")
    require(digest(canonical(finding)) == policy.get("finding_sha256"),
            "reviewed advisory JSON fingerprint differs")


def metadata_scope(metadata, root):
    """Retain every reverse edge, feature, source, kind and target; no host paths."""
    require(isinstance(metadata, dict) and metadata.get("version") == 1,
            "unsupported metadata format")
    packages = metadata.get("packages")
    resolve = metadata.get("resolve")
    require(isinstance(packages, list) and isinstance(resolve, dict), "full resolved metadata required")
    identities = {}
    for package in packages:
        mapping(package, "metadata package")
        source = package.get("source")
        if source is None:
            try:
                manifest = Path(package["manifest_path"]).resolve().relative_to(root.resolve()).as_posix()
            except (KeyError, ValueError) as error:
                raise PolicyError("workspace dependency outside the reviewed repository") from error
            source = f"workspace:{manifest}"
        identities[package["id"]] = f"{package['name']}@{package['version']}|{source}"
    require(len(identities) == len(packages), "duplicate metadata package identity")
    require(len(set(identities.values())) == len(packages), "ambiguous metadata package source")
    chrono = [package for package in packages if package.get("name") == "chrono"]
    require(len(chrono) == 1 and chrono[0].get("version") == "0.4.45"
            and chrono[0].get("source") == SOURCE, "unexpected chrono version/source")
    node_list = resolve.get("nodes")
    require(isinstance(node_list, list), "missing resolved nodes")
    nodes = {mapping(node, "resolved node")["id"]: node for node in node_list}
    require(len(nodes) == len(node_list), "duplicate resolved node")
    require(chrono[0]["id"] in nodes, "chrono missing from resolved graph")
    edges = []
    for identity, node in nodes.items():
        require(identity in identities and isinstance(node.get("features"), list)
                and all(isinstance(feature, str) for feature in node["features"])
                and isinstance(node.get("deps"), list), "malformed resolved metadata")
        for edge in node["deps"]:
            mapping(edge, "resolved edge")
            require(edge.get("pkg") in nodes and isinstance(edge.get("name"), str)
                    and isinstance(edge.get("dep_kinds"), list) and edge["dep_kinds"],
                    "missing resolved dependency kind/target")
            for kind in edge["dep_kinds"]:
                mapping(kind, "resolved dependency kind")
                require(set(kind) == {"kind", "target"} and kind["kind"] in (None, "build", "dev")
                        and (kind["target"] is None or isinstance(kind["target"], str)),
                        "unknown resolved dependency kind/target")
            edges.append((identity, edge))
    ancestors = {chrono[0]["id"]}
    while True:
        parents = {parent for parent, edge in edges if edge["pkg"] in ancestors}
        if parents <= ancestors:
            break
        ancestors.update(parents)
    return {
        "nodes": sorted(
            ({"package": identities[identity], "features": sorted(nodes[identity]["features"])}
             for identity in ancestors), key=lambda node: node["package"]),
        "edges": sorted(
            ({"parent": identities[parent], "dependency": identities[edge["pkg"]],
              "name": edge["name"],
              "kinds": sorted(edge["dep_kinds"], key=lambda kind: canonical(kind))}
             for parent, edge in edges if edge["pkg"] in ancestors), key=canonical),
    }


def application_uses(metadata, root):
    """Bind all Rust source in direct local parents and inventory references.

    The conservative full-source binding catches alias/re-export use changes
    within an unchanged direct parent. It also requires review of unrelated Rust
    edits in that one consumer crate. The supplementary lexical inventory is not
    whole-program proof; dependency integrity and source review remain required.
    """
    packages = {package["id"]: package for package in metadata["packages"]}
    chrono = next(package["id"] for package in packages.values() if package["name"] == "chrono")
    uses = {"files": {}, "references": []}
    for node in metadata["resolve"]["nodes"]:
        package = packages[node["id"]]
        if package.get("source") is not None or not any(edge["pkg"] == chrono for edge in node["deps"]):
            continue
        directory = Path(package["manifest_path"]).resolve().parent
        for current, directories, files in os.walk(directory):
            directories[:] = [name for name in directories if name not in ("target", ".git", ".local", "node_modules")]
            require(not any((Path(current) / name).is_symlink() for name in directories),
                    "unexpected source directory symlink in direct application consumer")
            for name in files:
                if not name.endswith(".rs"):
                    continue
                file = Path(current) / name
                content = file.read_bytes().replace(b"\r\n", b"\n")
                try:
                    file.resolve().relative_to(root.resolve())
                    relative = file.relative_to(root.resolve()).as_posix()
                except ValueError as error:
                    raise PolicyError("direct chrono-use file outside repository") from error
                uses["files"][relative] = digest(content)
                if re.search(r"\bchrono\b", content.decode("utf-8")):
                    uses["references"].append(relative)
    return {"files": dict(sorted(uses["files"].items())), "references": sorted(uses["references"])}


def validate_finding(finding, policy):
    mapping(finding, "scanner finding")
    reviewed = {key: finding.get(key) for key in ("advisory", "affected", "versions")}
    require(normalized(reviewed) == normalized(policy["reviewed_finding"]),
            "changed advisory classification, aliases, content or vulnerability scope")


def adjudicate_deny(stdout, stderr, status, policy):
    require(status == 1 and not stdout.strip(), "unexpected cargo-deny status/output")
    events = [strict_json(line) for line in stderr.splitlines() if line.strip()]
    errors, summaries = [], []
    for event in events:
        fields = mapping(event.get("fields"), "cargo-deny event fields")
        if event.get("type") == "diagnostic":
            severity = fields.get("severity")
            require(severity in ("error", "note", "help"), "unexpected blocking cargo-deny diagnostic")
            if severity == "error":
                errors.append(fields)
        elif event.get("type") == "summary":
            summaries.append(fields)
        elif event.get("type") == "log":
            require(fields.get("level") in ("INFO", "DEBUG", "TRACE"), "cargo-deny scanner failure/warning")
        else:
            raise PolicyError("unknown cargo-deny event")
    require(len(errors) == 1 and len(summaries) == 1, "extra/missing cargo-deny findings or summary")
    stats = mapping(summaries[0].get("advisories"), "cargo-deny summary statistics")
    require(set(summaries[0]) == {"advisories"} and stats.get("errors") == 1
            and stats.get("warnings") == 0, "cargo-deny summary does not match exact accepted finding")
    finding = errors[0]
    require(finding.get("code") == "unmaintained", "cargo-deny finding is not unmaintained")
    # cargo-deny 0.20.2 emits advisory metadata only; the fingerprinted
    # authoritative database bytes also bind versions and affected scope.
    require(normalized(finding.get("advisory")) == normalized(policy["reviewed_finding"]["advisory"]),
            "cargo-deny advisory differs from reviewed metadata")
    labels = finding.get("labels", [])
    require(len(labels) == 1 and labels[0].get("span") == f"chrono 0.4.45 {SOURCE}",
            "cargo-deny finding package/version/source mismatch")
    return NOTICE


def utc_instant(value):
    require(isinstance(value, str) and re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)", value),
        "missing or malformed timezone-aware database timestamp")
    try:
        return dt.datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(dt.timezone.utc)
    except ValueError as error:
        raise PolicyError("invalid database timestamp") from error


def audit_command(cargo, database, graph):
    require(graph in GRAPHS, "unapproved audit command graph")
    args = [cargo, "audit", "--json", "--no-fetch", "--db", str(database)]
    if graph == "root":
        args.extend(["-D", "warnings", "--ignore", RSA])
    return args + ["--file", str(Path(GRAPHS[graph]).with_name("Cargo.lock"))]


def validate_database_attestation(attestation, graph, policy):
    """Require independently captured fresh snapshot and exact scanner commands."""
    mapping(attestation, "fresh database/command attestation")
    before = mapping(attestation.get("before"), "before-scan database snapshot")
    after = mapping(attestation.get("after"), "after-scan database snapshot")
    require(before == after and before.get("clean") is True,
            "database attestation is not an unchanged clean snapshot")
    require(isinstance(before.get("commit"), str)
            and re.fullmatch(r"[0-9a-f]{40}", before["commit"])
            and isinstance(before.get("content_sha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", before["content_sha256"]),
            "missing authoritative database commit/content fingerprint")
    utc_instant(before.get("commit_timestamp"))
    database_path = attestation.get("database_path")
    config_path = attestation.get("config_path")
    cargo = attestation.get("cargo")
    require(isinstance(database_path, str) and Path(database_path).is_absolute()
            and before.get("database_path") == database_path
            and isinstance(config_path, str) and Path(config_path).is_absolute()
            and Path(config_path).name == "deny.toml"
            and Path(config_path).parent == Path(database_path).parent.parent
            and isinstance(cargo, str) and cargo, "database command path binding is missing")
    fetch = mapping(attestation.get("fetch"), "fresh database fetch status")
    require(type(fetch.get("exit")) is int and fetch["exit"] == 0
            and fetch.get("command") == [cargo, "deny", "--config", config_path, "fetch", "db"],
            "fresh database fetch command/status mismatch")
    require(attestation.get("audit_command") == audit_command(cargo, database_path, graph),
            "audit command does not attest --no-fetch and the exact shared database path")
    require(attestation.get("advisory_sha256") == policy["advisory_sha256"]
            and attestation.get("deny_config_sha256") == policy["deny_config_sha256"],
            "database attestation lacks reviewed advisory/configuration fingerprints")
    return before


def adjudicate_audit(stdout, stderr, status, graph, policy, attestation=None):
    require(graph in GRAPHS, "unapproved graph")
    require(not stderr.strip(), "cargo-audit scanner errors outside its JSON report")
    report = strict_json(stdout)
    require(set(report) == {"database", "lockfile", "settings", "vulnerabilities", "warnings"},
            "unknown or missing cargo-audit report fields")
    lockfile = mapping(report.get("lockfile"), "cargo-audit lockfile")
    require(type(lockfile.get("dependency-count")) is int and lockfile["dependency-count"] > 0,
            "unavailable cargo-audit lockfile evidence")
    settings = mapping(report.get("settings"), "cargo-audit settings")
    require(settings.get("ignore") == ([RSA] if graph == "root" else [])
            and settings.get("severity") is None and settings.get("target_arch") == []
            and settings.get("target_os") == []
            and set(settings.get("informational_warnings", [])) == {"unmaintained", "unsound", "notice"},
            "unexpected cargo-audit settings")
    database = mapping(report.get("database"), "cargo-audit database")
    require(set(database) == {"advisory-count", "last-commit", "last-updated"},
            "missing or unknown cargo-audit database fields")
    require(type(database.get("advisory-count")) is int and database["advisory-count"] > 0,
            "unavailable advisory database evidence")
    snapshot = validate_database_attestation(attestation, graph, policy)
    commit, timestamp = database["last-commit"], database["last-updated"]
    # cargo-audit 0.22.2 Auditor::new calls rustsec 0.33.0 Database::open
    # with --no-fetch. Database::open leaves latest_commit=None; DatabaseInfo
    # therefore emits an explicit null pair, even for an actual Git checkout.
    # Accept it only with the independent fresh/clean before-after snapshot,
    # exact --db command and fingerprints above. Missing/mixed pairs fail.
    if commit is not None or timestamp is not None:
        require(isinstance(commit, str) and isinstance(timestamp, str)
                and commit == snapshot["commit"], "cargo-audit database commit/pair mismatch")
        require(utc_instant(timestamp) == utc_instant(snapshot["commit_timestamp"]),
                "cargo-audit reported a different advisory database commit instant")
    vulnerabilities = mapping(report.get("vulnerabilities"), "cargo-audit vulnerabilities")
    require(vulnerabilities.get("found") is False and type(vulnerabilities.get("count")) is int
            and vulnerabilities["count"] == 0
            and vulnerabilities.get("list") == [], "unresolved cargo-audit vulnerability")
    warnings = report.get("warnings")
    require(isinstance(warnings, dict), "missing cargo-audit warnings")
    accepted = []
    for kind, findings in warnings.items():
        require(kind in ("unmaintained", "unsound", "notice", "yanked") and isinstance(findings, list),
                "unknown cargo-audit warning category")
        for finding in findings:
            mapping(finding, "cargo-audit warning")
            package = mapping(finding.get("package"), "cargo-audit package")
            require(finding.get("kind") == kind and isinstance(package.get("name"), str)
                    and isinstance(package.get("version"), str)
                    and (package.get("source") is None or isinstance(package.get("source"), str)),
                    "malformed cargo-audit warning identity")
            advisory = finding.get("advisory") or {}
            if kind == "yanked":
                require(finding.get("advisory") is None, "malformed yanked warning")
            else:
                mapping(advisory, "cargo-audit warning advisory")
                require(isinstance(advisory.get("id"), str) and advisory.get("informational") == kind
                        and advisory.get("package") == package["name"],
                        "malformed informational advisory")
            if package.get("name") == "chrono" or advisory.get("id") == ADVISORY:
                require(kind == "unmaintained" and finding.get("kind") == kind,
                        "chrono is no longer solely an unmaintained warning")
                require(all(package.get(key) == value for key, value in policy["package"].items())
                        and package.get("checksum") == policy["package_checksum"]
                        and package.get("replace") is None, "cargo-audit chrono identity mismatch")
                validate_finding(finding, policy)
                accepted.append(finding)
            else:
                # Desktop historically reports informational/yanked warnings without
                # -D warnings. Root retains -D warnings and rejects every other one.
                require(graph == "desktop", "unresolved cargo-audit warning")
    require(len(accepted) == 1 and status == (1 if graph == "root" else 0),
            "missing/duplicate chrono finding or unknown cargo-audit exit")
    return NOTICE


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def run_raw(root, evidence, name, args, emit=True):
    """Save and prominently emit original bytes/status before any adjudication."""
    print("+ " + " ".join(str(arg) for arg in args), flush=True)
    environment = os.environ.copy()
    if args[0] == "git":
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES"):
            environment.pop(key, None)
    try:
        result = subprocess.run(args, cwd=root, env=environment, capture_output=True, timeout=900, check=False)
    except subprocess.TimeoutExpired as error:
        result = subprocess.CompletedProcess(args, None, error.stdout or b"", error.stderr or b"")
    (evidence / f"{name}.stdout").write_bytes(result.stdout)
    (evidence / f"{name}.stderr").write_bytes(result.stderr)
    save_json(evidence / f"{name}.status.json", {"command": [str(arg) for arg in args], "exit": result.returncode})
    print(f"RAW UNFILTERED {name} diagnostics (saved in {evidence}):", flush=True)
    if emit:
        sys.stdout.buffer.write(result.stdout)
        sys.stdout.buffer.flush()
        sys.stderr.buffer.write(result.stderr)
        sys.stderr.buffer.flush()
    return result


def successful(result, label):
    require(result.returncode == 0, f"{label} failed; unavailable data is unverified")
    return result.stdout.decode("utf-8")


def snapshot_state(root, evidence, database, suffix):
    state = {"database_path": str(database.resolve())}
    for name, args in {
        "commit": ["rev-parse", "HEAD"],
        "commit_timestamp": ["show", "-s", "--format=%cI", "HEAD"],
        "dirty": ["status", "--porcelain", "--untracked-files=all"],
    }.items():
        result = run_raw(root, evidence, f"database-{name}-{suffix}", ["git", "-C", str(database), *args])
        state[name] = successful(result, "advisory snapshot provenance").strip()
    require(not state.pop("dirty"), "advisory database is not a clean authoritative snapshot")
    state["clean"] = True
    state["content_sha256"] = digest(canonical([
        (file.relative_to(database).as_posix(), digest(file.read_bytes()))
        for file in sorted(database.rglob("*.md")) if ".git" not in file.relative_to(database).parts
    ]))
    return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root-only", action="store_true", help="preserve the release-readiness root advisory scope")
    arguments = parser.parse_args()
    graphs = {"root": GRAPHS["root"]} if arguments.root_only else GRAPHS
    root = Path(__file__).resolve().parents[2]
    evidence = root / ".local" / "dependency-policy" / (dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    evidence.mkdir(parents=True, exist_ok=False)
    receipt = {"outcome": "failed", "evidence": str(evidence), "graphs": list(graphs), "scanned_at_utc": dt.datetime.now(dt.timezone.utc).isoformat()}
    print(f"Dependency-policy raw evidence and disposition receipt: {evidence}", flush=True)
    try:
        policy_bytes = (root / POLICY).read_bytes()
        policy = strict_json(policy_bytes)
        receipt["policy_sha256"] = digest(policy_bytes)
        receipt["implementation_sha256"] = {
            path: digest((root / path).read_bytes().replace(b"\r\n", b"\n"))
            for path in ("scripts/ci/dependency-policy.py", "scripts/ci/test-dependency-policy.py", "xtask/src/checks/surfaces.rs")
        }
        config = (root / "deny.toml").read_bytes().replace(b"\r\n", b"\n")
        require(digest(config) == policy["deny_config_sha256"], "deny.toml changed; review existing exception semantics")
        require(config.count(b"[advisories]\n") == 1, "ambiguous advisory configuration")
        database_root = evidence / "advisory-dbs"
        config_path = evidence / "deny.toml"
        config_path.write_bytes(config.replace(b"[advisories]\n", ("[advisories]\ndb-path = " + json.dumps(str(database_root)) + "\n").encode()))
        cargo = os.environ.get("CARGO", "cargo")
        fetch_result = run_raw(root, evidence, "fetch-database", [cargo, "deny", "--config", str(config_path), "fetch", "db"])
        successful(fetch_result, "fresh advisory fetch")
        databases = [path for path in database_root.iterdir() if path.is_dir()]
        require(len(databases) == 1, "expected one authoritative advisory database")
        database = databases[0]
        receipt["database"] = snapshot_state(root, evidence, database, "before")
        advisory = (database / "crates" / "chrono" / f"{ADVISORY}.md").read_bytes()
        validate_policy(policy, advisory, dt.datetime.now(dt.timezone.utc))
        lock_hashes = {graph: digest((root / Path(manifest).with_name("Cargo.lock")).read_bytes()) for graph, manifest in graphs.items()}
        receipt["lock_sha256"] = lock_hashes
        metadatas = {}
        for graph, manifest in graphs.items():
            metadata_result = run_raw(root, evidence, f"{graph}-metadata", [cargo, "metadata", "--locked", "--all-features", "--manifest-path", manifest, "--format-version", "1"], emit=False)
            metadata = strict_json(successful(metadata_result, f"{graph} metadata"))
            require(metadata_scope(metadata, root) == policy["graphs"][graph], f"{graph} parent/use/source/features scope changed")
            require(application_uses(metadata, root) == policy["application_uses"][graph], f"{graph} direct application chrono-use files changed")
            metadatas[graph] = metadata
        deny = run_raw(root, evidence, "root-deny-advisories", [cargo, "deny", "--format", "json", "--config", str(config_path), "--locked", "--offline", "check", "-D", "warnings", "advisories"])
        # Preserve both scanners' original diagnostics even when either fails policy.
        audits = {}
        for graph, manifest in graphs.items():
            audits[graph] = run_raw(root, evidence, f"{graph}-audit", audit_command(cargo, database, graph))
        after = snapshot_state(root, evidence, database, "after")
        require(after == receipt["database"], "advisory snapshot changed during scans")
        require(lock_hashes == {graph: digest((root / Path(manifest).with_name("Cargo.lock")).read_bytes()) for graph, manifest in graphs.items()}, "lockfile changed during scans")
        require(all(application_uses(metadata, root) == policy["application_uses"][graph] for graph, metadata in metadatas.items()), "direct application chrono uses changed during scans")
        validate_policy(policy, advisory, dt.datetime.now(dt.timezone.utc))
        print(adjudicate_deny(deny.stdout, deny.stderr, deny.returncode, policy), flush=True)
        receipt["audit_database_attestations"] = {}
        for graph, audit in audits.items():
            attestation = {
                "before": receipt["database"], "after": after,
                "database_path": str(database.resolve()), "config_path": str(config_path.resolve()), "cargo": cargo,
                "fetch": {"command": fetch_result.args, "exit": fetch_result.returncode},
                "audit_command": audit.args,
                "advisory_sha256": digest(advisory.replace(b"\r\n", b"\n")),
                "deny_config_sha256": digest(config),
            }
            receipt["audit_database_attestations"][graph] = attestation
            print(f"{graph}: " + adjudicate_audit(audit.stdout, audit.stderr, audit.returncode, graph, policy, attestation), flush=True)
        receipt["outcome"] = "accepted_temporary_support_risk"
        receipt["notice"] = NOTICE
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
                summary.write(f"\n**{NOTICE}**\n\nRaw unfiltered scanner evidence: `{evidence}`.\n")
        return 0
    except (PolicyError, OSError, KeyError, TypeError, UnicodeError, subprocess.SubprocessError) as error:
        receipt["error"] = str(error)
        print(f"Dependency policy BLOCKED: {error}", file=sys.stderr, flush=True)
        return 1
    finally:
        save_json(evidence / "policy-receipt.json", receipt)


if __name__ == "__main__":
    sys.exit(main())
