#!/usr/bin/env python3
"""Failure-boundary fixtures for the temporary chrono support-risk approval."""

import copy
import datetime as dt
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("dependency-policy.py")
SPEC = importlib.util.spec_from_file_location("dependency_policy", SCRIPT)
POLICY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(POLICY)
ROOT = SCRIPT.resolve().parents[2]


class DependencyPolicyTests(unittest.TestCase):
    def setUp(self):
        self.policy = json.loads((ROOT / POLICY.POLICY).read_bytes())
        self.advisory = (ROOT / "ci/dependency-policy/reviewed-advisory.md").read_bytes()
        self.now = dt.datetime(2026, 11, 19, tzinfo=dt.timezone.utc)

    def validate(self, policy=None, advisory=None, now=None):
        POLICY.validate_policy(policy or self.policy, advisory or self.advisory, now or self.now)

    def warning(self):
        return {
            **copy.deepcopy(self.policy["reviewed_finding"]), "kind": "unmaintained",
            "package": {**self.policy["package"], "checksum": self.policy["package_checksum"], "replace": None},
        }

    def audit(self, graph="root"):
        return {
            # Actual --no-fetch database object from both preserved failed-run
            # .local/dependency-policy/20261119T025420Z-7756f482/*-audit.stdout.
            "database": {"advisory-count": 1533, "last-commit": None, "last-updated": None},
            "lockfile": {"dependency-count": 1},
            "settings": {"ignore": [POLICY.RSA] if graph == "root" else [], "severity": None,
                         "target_arch": [], "target_os": [],
                         "informational_warnings": ["unmaintained", "unsound", "notice"]},
            "vulnerabilities": {"found": False, "count": 0, "list": []},
            "warnings": {"unmaintained": [self.warning()]},
        }

    def deny(self):
        return [
            {"type": "diagnostic", "fields": {
                "severity": "error", "code": "unmaintained",
                "advisory": copy.deepcopy(self.policy["reviewed_finding"]["advisory"]),
                "labels": [{"span": f"chrono 0.4.45 {POLICY.SOURCE}"}],
            }},
            {"type": "summary", "fields": {"advisories": {"errors": 1, "warnings": 0, "notes": 0, "helps": 0}}},
        ]

    def check_audit(self, report, graph="root", status=None, stderr=b""):
        if status is None:
            status = 1 if graph == "root" else 0
        return POLICY.adjudicate_audit(json.dumps(report).encode(), stderr, status, graph, self.policy,
                                       self.attestation(graph))

    def attestation(self, graph="root"):
        database = ROOT / ".local/fixture-policy/advisory-dbs/advisory-db-reviewed"
        config = database.parent.parent / "deny.toml"
        before = {"commit": "472a4c9a785030434f15d7da84964d60fbe789ab",
                  "commit_timestamp": "2026-11-18T10:14:42+01:00", "content_sha256": "a" * 64,
                  "database_path": str(database), "clean": True}
        return {"before": before, "after": copy.deepcopy(before), "database_path": str(database),
                "config_path": str(config), "cargo": "cargo",
                "fetch": {"command": ["cargo", "deny", "--config", str(config), "fetch", "db"], "exit": 0},
                "audit_command": POLICY.audit_command("cargo", database, graph),
                "advisory_sha256": self.policy["advisory_sha256"],
                "deny_config_sha256": self.policy["deny_config_sha256"]}

    def check_deny(self, events, status=1):
        return POLICY.adjudicate_deny(b"", "\n".join(json.dumps(event) for event in events).encode(), status, self.policy)

    def test_exact_acceptance_remains_visibly_reported(self):
        self.validate()
        for notice in [self.check_audit(self.audit()), self.check_audit(self.audit("desktop"), "desktop"), self.check_deny(self.deny())]:
            self.assertIn("accepted temporary support risk", notice)
            self.assertIn(POLICY.ADVISORY, notice)
            self.assertIn(POLICY.EXPIRY, notice)
            self.assertIn("not a chrono fix", notice)

    def test_expiry_boundary_and_missing_or_changed_approval_fail(self):
        for moment in [dt.datetime(2026, 12, 19, tzinfo=dt.timezone.utc), dt.datetime(2026, 12, 20, tzinfo=dt.timezone.utc)]:
            with self.subTest(moment=moment), self.assertRaises(POLICY.PolicyError):
                self.validate(now=moment)
        self.validate(now=dt.datetime(2026, 12, 18, 23, 59, 59, tzinfo=dt.timezone.utc))
        for value in [None, {}, {**POLICY.APPROVAL, "expires_utc": "2027-01-01T00:00:00Z"}]:
            changed = copy.deepcopy(self.policy)
            changed["approval"] = value
            with self.subTest(approval=value), self.assertRaises(POLICY.PolicyError):
                self.validate(policy=changed)
        for field in ("removal", "rationale"):
            changed = copy.deepcopy(self.policy)
            del changed[field]
            with self.subTest(missing=field), self.assertRaises(POLICY.PolicyError):
                self.validate(policy=changed)

    def test_policy_package_version_source_graph_and_current_advisory_are_exact(self):
        for field, value in [("name", "chrono-fork"), ("version", "0.4.46"), ("source", "git+https://example.invalid/chrono")]:
            changed = copy.deepcopy(self.policy)
            changed["package"][field] = value
            with self.subTest(field=field), self.assertRaises(POLICY.PolicyError):
                self.validate(policy=changed)
        changed = copy.deepcopy(self.policy)
        changed["graphs"]["fuzz"] = changed["graphs"]["root"]
        with self.assertRaises(POLICY.PolicyError):
            self.validate(policy=changed)
        with self.assertRaises(POLICY.PolicyError):
            self.validate(advisory=self.advisory + b"\nNew security information.\n")

    def test_changed_classification_alias_content_version_or_affected_scope_fails(self):
        mutations = [
            lambda finding: finding["advisory"].update(informational="unsound"),
            lambda finding: finding["advisory"].update(aliases=["CVE-2026-00001"]),
            lambda finding: finding["advisory"].update(description="Changed upstream content"),
            lambda finding: finding["versions"].update(patched=[">=0.4.46"]),
            lambda finding: finding.update(affected={"os": ["windows"]}),
            lambda finding: finding["package"].update(version="0.4.46"),
            lambda finding: finding["package"].update(source="registry+https://example.invalid/index"),
        ]
        for mutate in mutations:
            report = self.audit()
            mutate(report["warnings"]["unmaintained"][0])
            with self.subTest(mutation=mutate), self.assertRaises(POLICY.PolicyError):
                self.check_audit(report)

    def test_new_vulnerability_and_root_warning_remain_blocking(self):
        for graph in POLICY.GRAPHS:
            report = self.audit(graph)
            report["vulnerabilities"] = {"found": True, "count": 1, "list": [self.warning()]}
            with self.subTest(graph=graph), self.assertRaises(POLICY.PolicyError):
                self.check_audit(report, graph)
        report = self.audit()
        extra = self.warning()
        extra["package"]["name"] = "another-crate"
        extra["advisory"]["id"] = "RUSTSEC-2026-9999"
        extra["advisory"]["package"] = "another-crate"
        report["warnings"]["unmaintained"].append(extra)
        with self.assertRaises(POLICY.PolicyError):
            self.check_audit(report)
        # Preserve Desktop's existing non-D-warnings semantics, visibly reported
        # by its unfiltered scanner report. This approval does not ignore it.
        desktop = self.audit("desktop")
        desktop["warnings"]["unmaintained"].append(extra)
        self.assertEqual(self.check_audit(desktop, "desktop"), POLICY.NOTICE)
        desktop["warnings"]["unmaintained"].append({"package": {"name": "broken"}})
        with self.assertRaises(POLICY.PolicyError):
            self.check_audit(desktop, "desktop")

    def test_deny_extra_diagnostic_summary_classification_and_source_fail(self):
        cases = []
        extra = self.deny()
        extra.insert(0, {"type": "diagnostic", "fields": {"severity": "error", "code": "vulnerability"}})
        cases.append(extra)
        warning = self.deny()
        warning.insert(0, {"type": "log", "fields": {"level": "ERROR", "message": "database unavailable"}})
        cases.append(warning)
        cases.append(self.deny()[:-1])
        changed = self.deny()
        changed[0]["fields"]["advisory"]["aliases"] = ["GHSA-new-information"]
        cases.append(changed)
        changed = self.deny()
        changed[0]["fields"]["labels"][0]["span"] = "chrono 0.4.46 " + POLICY.SOURCE
        cases.append(changed)
        for events in cases:
            with self.subTest(events=events), self.assertRaises(POLICY.PolicyError):
                self.check_deny(events)

    def test_malformed_missing_scanner_data_and_unknown_status_fail_closed(self):
        with self.assertRaises(POLICY.PolicyError):
            POLICY.validate_policy({}, self.advisory, self.now)
        for raw in [b"", b"not json", b"[]", b'{"settings":[]}', b'{"warnings":{},"warnings":{}}', b'{"x":NaN}']:
            with self.subTest(raw=raw), self.assertRaises(POLICY.PolicyError):
                POLICY.adjudicate_audit(raw, b"", 1, "root", self.policy)
        for status in [0, 2, 101, -1]:
            with self.subTest(status=status), self.assertRaises(POLICY.PolicyError):
                self.check_audit(self.audit(), status=status)
            with self.assertRaises(POLICY.PolicyError):
                self.check_deny(self.deny(), status=status)
        with self.assertRaises(POLICY.PolicyError):
            POLICY.adjudicate_audit(json.dumps(self.audit()).encode(), b"", None, "root", self.policy,
                                    self.attestation())
        with self.assertRaises(POLICY.PolicyError):
            self.check_audit(self.audit(), stderr=b"error: failed to load advisory database")
        with self.assertRaises(POLICY.PolicyError):
            self.check_audit(self.audit(), graph="fuzz")
        with self.assertRaises(POLICY.PolicyError):
            POLICY.adjudicate_audit(json.dumps(self.audit()).encode(), b"", 1, "root", self.policy,
                                    {"commit": "different", "commit_timestamp": "unavailable"})
        unavailable = self.audit()
        unavailable["database"]["advisory-count"] = 0
        with self.assertRaises(POLICY.PolicyError):
            self.check_audit(unavailable)

    def test_actual_no_fetch_null_pair_requires_strong_database_and_command_attestation(self):
        report = self.audit()
        self.assertIsNone(report["database"]["last-commit"])
        self.assertIsNone(report["database"]["last-updated"])
        self.assertEqual(self.check_audit(report), POLICY.NOTICE)
        with self.assertRaises(POLICY.PolicyError):
            POLICY.adjudicate_audit(json.dumps(report).encode(), b"", 1, "root", self.policy)
        mutations = [
            lambda data: data["fetch"].update(exit=1),
            lambda data: data["fetch"]["command"].append("unreviewed-argument"),
            lambda data: data["after"].update(commit="f" * 40),
            lambda data: data["after"].update(content_sha256="f" * 64),
            lambda data: data["before"].update(clean=False),
            lambda data: data.update(advisory_sha256="f" * 64),
            lambda data: data.update(deny_config_sha256="f" * 64),
            lambda data: data["audit_command"].remove("--json"),
            lambda data: data["audit_command"].remove("--no-fetch"),
            lambda data: data["audit_command"].__setitem__(5, str(ROOT / ".local/other-db")),
            lambda data: data.pop("before"),
            lambda data: data.pop("fetch"),
        ]
        for mutate in mutations:
            attestation = self.attestation()
            mutate(attestation)
            with self.subTest(mutation=mutate), self.assertRaises(POLICY.PolicyError):
                POLICY.adjudicate_audit(json.dumps(report).encode(), b"", 1, "root", self.policy, attestation)

    def test_generated_audit_commands_match_independent_literal_graph_flags(self):
        database = str(ROOT / ".local/fixture-policy/advisory-dbs/advisory-db-reviewed")
        self.assertEqual(POLICY.audit_command("cargo", database, "root"), [
            "cargo", "audit", "--json", "--no-fetch", "--db", database,
            "-D", "warnings", "--ignore", "RUSTSEC-2023-0071", "--file", "Cargo.lock",
        ])
        self.assertEqual(POLICY.audit_command("cargo", database, "desktop"), [
            "cargo", "audit", "--json", "--no-fetch", "--db", database,
            "--file", str(Path("apps/desktop/src-tauri/Cargo.lock")),
        ])
        root_without_threshold = self.attestation()
        root_without_threshold["audit_command"].remove("-D")
        root_with_chrono_ignore = self.attestation()
        command = root_with_chrono_ignore["audit_command"]
        command[command.index("--ignore") + 1] = "RUSTSEC-2026-0543"
        desktop_with_threshold = self.attestation("desktop")
        desktop_with_threshold["audit_command"].extend(["-D", "warnings"])
        for graph, attestation in [("root", root_without_threshold), ("root", root_with_chrono_ignore),
                                   ("desktop", desktop_with_threshold)]:
            with self.subTest(graph=graph, command=attestation["audit_command"]), self.assertRaises(POLICY.PolicyError):
                POLICY.adjudicate_audit(json.dumps(self.audit(graph)).encode(), b"", 1 if graph == "root" else 0,
                                        graph, self.policy, attestation)

    def test_database_missing_mixed_or_wrong_metadata_fails(self):
        cases = []
        for field in ("last-commit", "last-updated"):
            report = self.audit()
            del report["database"][field]
            cases.append(report)
        for commit, timestamp in [
            ("472a4c9a785030434f15d7da84964d60fbe789ab", None),
            (None, "2026-11-18T10:14:42+01:00"),
            ("f" * 40, "2026-11-18T10:14:42+01:00"),
            ("472a4c9a785030434f15d7da84964d60fbe789ab", "2026-11-18T10:14:43+01:00"),
            ("472a4c9a785030434f15d7da84964d60fbe789ab", "2026-11-18T09:14:42"),
            ("472a4c9a785030434f15d7da84964d60fbe789ab", "2026-11-18T09:14:42+99:99"),
        ]:
            report = self.audit()
            report["database"].update({"last-commit": commit, "last-updated": timestamp})
            cases.append(report)
        for report in cases:
            with self.subTest(database=report["database"]), self.assertRaises(POLICY.PolicyError):
                self.check_audit(report)

    def test_database_same_instant_offset_variants_are_accepted(self):
        for timestamp in ("2026-11-18T10:14:42+01:00", "2026-11-18T04:14:42-05:00", "2026-11-18T09:14:42Z"):
            report = self.audit()
            report["database"].update({"last-commit": self.attestation()["before"]["commit"], "last-updated": timestamp})
            with self.subTest(timestamp=timestamp):
                self.assertEqual(self.check_audit(report), POLICY.NOTICE)

    def test_scope_binds_parent_source_version_features_kind_target_and_use(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            chrono_id, parent_id = "registry-chrono", "local-parent"
            metadata = {
                "version": 1,
                "packages": [
                    {"id": chrono_id, "name": "chrono", "version": "0.4.45", "source": POLICY.SOURCE},
                    {"id": parent_id, "name": "consumer", "version": "0.12.1", "source": None,
                     "manifest_path": str(root / "crates/consumer/Cargo.toml")},
                ],
                "resolve": {"nodes": [
                    {"id": chrono_id, "features": ["std"], "deps": []},
                    {"id": parent_id, "features": [], "deps": [
                        {"name": "chrono", "pkg": chrono_id, "dep_kinds": [{"kind": None, "target": None}]}]},
                ]},
            }
            approved = POLICY.metadata_scope(metadata, root)
            self.assertIn("workspace:crates/consumer/Cargo.toml", approved["nodes"][1]["package"])
            changes = [
                lambda data: data["packages"][1].update(version="0.12.2"),
                lambda data: data["packages"][1].update(source="registry+https://example.invalid/index"),
                lambda data: data["resolve"]["nodes"][0]["features"].append("clock"),
                lambda data: data["resolve"]["nodes"][1]["deps"][0]["dep_kinds"][0].update(kind="build"),
                lambda data: data["resolve"]["nodes"][1]["deps"][0]["dep_kinds"][0].update(target="cfg(windows)"),
            ]
            for mutate in changes:
                changed = copy.deepcopy(metadata)
                mutate(changed)
                self.assertNotEqual(POLICY.metadata_scope(changed, root), approved)
            changed = copy.deepcopy(metadata)
            changed["packages"].append({"id": "new", "name": "new-consumer", "version": "1.0.0", "source": POLICY.SOURCE})
            changed["resolve"]["nodes"].append({"id": "new", "features": [], "deps": [{"name": "parent", "pkg": parent_id, "dep_kinds": [{"kind": "dev", "target": None}]}]})
            self.assertNotEqual(POLICY.metadata_scope(changed, root), approved)
            with self.assertRaises(POLICY.PolicyError):
                POLICY.metadata_scope({"version": 1, "packages": [], "resolve": None}, root)
            duplicate = copy.deepcopy(metadata)
            duplicate["resolve"]["nodes"].append(copy.deepcopy(duplicate["resolve"]["nodes"][0]))
            with self.assertRaises(POLICY.PolicyError):
                POLICY.metadata_scope(duplicate, root)
            missing_kind = copy.deepcopy(metadata)
            missing_kind["resolve"]["nodes"][1]["deps"][0]["dep_kinds"] = []
            with self.assertRaises(POLICY.PolicyError):
                POLICY.metadata_scope(missing_kind, root)

    def test_same_dependency_graph_changed_existing_use_and_new_reference_file_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "crates/consumer"
            source = package / "src"
            source.mkdir(parents=True)
            existing = source / "auth.rs"
            existing.write_text("fn now() { chrono::Utc::now(); }\n", encoding="utf-8")
            metadata = {
                "packages": [
                    {"id": "chrono", "name": "chrono", "source": POLICY.SOURCE},
                    {"id": "consumer", "name": "consumer", "source": None,
                     "manifest_path": str(package / "Cargo.toml")},
                ],
                "resolve": {"nodes": [
                    {"id": "chrono", "deps": []},
                    {"id": "consumer", "deps": [{"pkg": "chrono"}]},
                ]},
            }
            approved = POLICY.application_uses(metadata, root)
            self.assertEqual(list(approved["files"]), ["crates/consumer/src/auth.rs"])
            self.assertEqual(approved["references"], ["crates/consumer/src/auth.rs"])
            existing.write_text("fn now() { chrono::Local::now(); }\n", encoding="utf-8")
            self.assertNotEqual(POLICY.application_uses(metadata, root), approved)
            existing.write_text("fn now() { chrono::Utc::now(); }\n", encoding="utf-8")
            new_use = source / "new_use.rs"
            new_use.write_text("extern crate chrono;\n", encoding="utf-8")
            self.assertNotEqual(POLICY.application_uses(metadata, root), approved)
            new_use.unlink()
            (source / "unrelated.rs").write_text("fn unrelated() {}\n", encoding="utf-8")
            # The approved conservative guard freezes this direct consumer's
            # Rust source, catching alias/re-export changes without a new token.
            self.assertNotEqual(POLICY.application_uses(metadata, root), approved)
            (source / "unrelated.rs").unlink()
            ignored = package / "target"
            ignored.mkdir()
            (ignored / "generated.rs").write_text("chrono::Utc::now();\n", encoding="utf-8")
            self.assertEqual(POLICY.application_uses(metadata, root), approved)
            existing.unlink()
            self.assertNotEqual(POLICY.application_uses(metadata, root), approved)


if __name__ == "__main__":
    unittest.main()
