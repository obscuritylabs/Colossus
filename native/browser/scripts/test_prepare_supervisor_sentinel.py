"""Portable contracts only: no native image, subprocess, Mach or store is used."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import prepare_supervisor_sentinel as fixture

class SupervisorFixtureContracts(unittest.TestCase):
    def markers(self, root):
        (root/'custody').mkdir(mode=0o700)
        (root/'allocation/profile').mkdir(parents=True, mode=0o700)
        for name in fixture.fronts.MARKERS:
            path=root/'custody'/name
            path.write_bytes(b'colossus public custody probe v1\n')
            path.chmod(0o600)
    def test_only_reviewed_public_markers_are_retired_before_real_creator(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp).resolve(); self.markers(root)
            retained=root/'custody/unrelated'; retained.write_bytes(b'retain arbitrary file')
            fixture.retire_front_probe_markers(root)
            self.assertTrue(all(not (root/'custody'/name).exists() for name in fixture.fronts.MARKERS))
            self.assertEqual(retained.read_bytes(),b'retain arbitrary file')
    def test_changed_marker_bytes_or_symlink_prevent_any_marker_removal(self):
        for invalid in ('bytes','symlink'):
            with tempfile.TemporaryDirectory() as temp:
                root=Path(temp).resolve(); self.markers(root)
                path=root/'custody'/fixture.fronts.MARKERS[2]
                if invalid=='bytes':path.write_bytes(b'unreviewed marker')
                else:
                    path.unlink(); path.symlink_to(root/'custody'/fixture.fronts.MARKERS[0])
                with self.subTest(invalid=invalid),self.assertRaises((OSError,ValueError)):
                    fixture.retire_front_probe_markers(root)
                self.assertTrue((root/'custody'/fixture.fronts.MARKERS[0]).exists())
    def evidence(self, case="parent-crash"):
        crash = case in ("parent-crash", "parent-crash-output-loss")
        return dict(case=case, accepted=True, core_reaped=True, core_crashed_without_drop=crash,
                    core_exit_code=73 if crash else 0, creator_exit_observed=crash, keeper_exit_observed=crash,
                    creator_exit_code=20, keeper_exit_code=20, creator_reaped_by_core=not crash,
                    creator_exit_signal=None, keeper_exit_signal=None,
                    keeper_reaped_by_core=not crash, known_children_closed=True, helpers_reaped=4,
                    output_eof=True, forced=False, full_helper_cleanup_proven=False, production_enabled=False)
    def test_only_four_exact_cases_are_available(self):
        self.assertEqual(fixture.CASES, ("cancel", "eof", "parent-crash", "parent-crash-output-loss"))
        for case in fixture.CASES:
            fixture.verify_result(case, self.evidence(case))
        with self.assertRaises(ValueError):
            fixture.verify_result("arbitrary", self.evidence())
    def test_output_eof_or_signals_cannot_replace_receipt_and_kernel_exit(self):
        for field, value in [("known_children_closed",False),("helpers_reaped",3),("output_eof",False),
                             ("creator_exit_observed",False),("keeper_exit_observed",False),
                             ("creator_exit_code",None),("creator_exit_signal",6),("keeper_exit_signal",13),("core_exit_code",0),("forced",True)]:
            evidence=self.evidence(); evidence[field]=value
            with self.subTest(field=field), self.assertRaises(ValueError):
                fixture.verify_result("parent-crash",evidence)
    def test_nonchild_observation_never_becomes_a_parent_reap_claim(self):
        for field in ("creator_reaped_by_core","keeper_reaped_by_core","full_helper_cleanup_proven","production_enabled"):
            evidence=self.evidence(); evidence[field]=True
            with self.subTest(field=field), self.assertRaises(ValueError):
                fixture.verify_result("parent-crash",evidence)
    def test_run_derives_fixed_controller_command_instead_of_manifest_command(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp).resolve(); inventory=root/'inventory.json'; inventory.write_text('fixed-public-inventory')
            prepared=root/'prepared.json'
            prepared.write_text(json.dumps(dict(schema_version=1,purpose='colossus-known-supervisor-sentinel-prepared-v1',
                root=str(root),inventory_sha256=fixture.digest(inventory),source_sha256={'source':'bound'},
                production_enabled=False,full_helper_cleanup_proven=False,commands={'parent-crash':['untrusted','command']})))
            target=root/'case-parent-crash';target.mkdir();(target/'result.json').write_text(json.dumps(self.evidence()))
            with mock.patch.object(fixture,'sources',return_value={'source':'bound'}), mock.patch.object(fixture,'tool') as tool:
                fixture.run(prepared,'parent-crash')
                self.assertEqual(tool.call_args.args[0],[str(root/'controller'),'parent-crash'])
    def test_changed_reviewed_sources_prevent_any_launch(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp).resolve();inventory=root/'inventory.json';inventory.write_text('public')
            prepared=root/'prepared.json';prepared.write_text(json.dumps(dict(schema_version=1,purpose='colossus-known-supervisor-sentinel-prepared-v1',root=str(root),inventory_sha256=fixture.digest(inventory),source_sha256={},production_enabled=False,full_helper_cleanup_proven=False)))
            with mock.patch.object(fixture,'sources',return_value={'changed':'source'}),mock.patch.object(fixture,'tool') as tool:
                with self.assertRaises(ValueError):fixture.run(prepared,'eof')
                tool.assert_not_called()
if __name__=='__main__':unittest.main()
