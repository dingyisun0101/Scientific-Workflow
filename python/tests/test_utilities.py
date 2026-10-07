import json
import tempfile
import unittest
from pathlib import Path
from snapshot_fixture import seal_inputs
from scientific_workflow.dependencies import Dependencies, DependencyError, AmbiguousDependencyError
from scientific_workflow.project import parameters, study_path, ProjectLayoutError


class UtilitiesTests(unittest.TestCase):
    def test_dependency_cardinality_extensions_and_artifact_paths(self):
        value = [{"phase": phase, "tasks": [{"identity": "t", "output_directory": f"/run/{phase}", "workload": {"kind": "program", "executable": "/bin/sh"}}]} for phase in ("a", "b")]
        deps = Dependencies(value)
        with self.assertRaises(AmbiguousDependencyError): deps.programs().one()
        self.assertEqual(deps.programs().in_phase("a").one().directory, Path("/run/a/artifacts"))
        self.assertIsNone(deps.recordings().optional())
        copy = deps.raw_json(); copy.clear()
        self.assertEqual(len(tuple(deps.programs())), 2)
        value[0]["tasks"][0]["workload"]["kind"] = "future"
        self.assertEqual(len(tuple(Dependencies(value).programs())), 1)
        value[1]["tasks"][0]["workload"]["executable"] = "relative"
        with self.assertRaises(DependencyError): Dependencies(value)

    def test_parameters_use_resolved_snapshot_and_layout_errors_name_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / "workflow-config.json"
            snapshot.write_text(json.dumps({"config": {"parameters.json": {"analysis": {"bins": 3}}}}))
            seal_inputs(root)
            self.assertEqual(parameters("analysis", snapshot=snapshot), {"bins": 3})
            with self.assertRaisesRegex(ProjectLayoutError, "wf_configs/study.json"): study_path(root)
            with self.assertRaisesRegex(ProjectLayoutError, "missing"): parameters("missing", snapshot=snapshot)

class SnapshotIntegrityTests(unittest.TestCase):
    def test_either_captured_file_change_fails_both_accessors(self):
        for name in ("workflow-config.json", "workflow-dependencies.json"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                seal_inputs(root)
                (root / name).write_bytes((root / name).read_bytes() + b"\n")
                with self.assertRaisesRegex(ProjectLayoutError, "checksum mismatch"):
                    parameters(snapshot=root / "workflow-config.json")
                with self.assertRaisesRegex(DependencyError, "checksum mismatch"):
                    Dependencies.load(root / "workflow-dependencies.json")

    def test_current_program_and_receipt_anchors_are_required_when_present(self):
        for name, current, old in (("program.json", "scientific-workflow-program-v2", "scientific-workflow-program-v1"),
                                  ("workflow-result.json", "scientific-workflow-task-result.v2", "scientific-workflow-task-result.v1")):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                refs = seal_inputs(root)
                anchor = root / name
                anchor.write_text(json.dumps({"format": current, "inputs": refs}))
                self.assertEqual(parameters(snapshot=root / "workflow-config.json"), {})
                self.assertEqual(tuple(Dependencies.load(root / "workflow-dependencies.json").recordings()), ())
                anchor.write_text(json.dumps({"format": old, "inputs": refs}))
                with self.assertRaises(ProjectLayoutError):
                    parameters(snapshot=root / "workflow-config.json")
                anchor.write_text(json.dumps({"format": current}))
                with self.assertRaises(DependencyError):
                    Dependencies.load(root / "workflow-dependencies.json")

    def test_changed_snapshot_and_sidecar_cannot_override_program_anchor(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            refs = seal_inputs(root)
            (root / "program.json").write_text(json.dumps({"format": "scientific-workflow-program-v2", "inputs": refs}))
            config = root / "workflow-config.json"
            config.write_text(json.dumps({"config": {"parameters.json": {"changed": True}}}))
            seal_inputs(root)
            with self.assertRaisesRegex(ProjectLayoutError, "descriptors differ"):
                parameters(snapshot=config)

    def test_missing_malformed_and_escaping_snapshot_evidence_is_rejected(self):
        for mutation in ("missing", "unknown", "format", "checksum", "path"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                seal_inputs(root)
                metadata = root / "workflow-inputs.json"
                if mutation == "missing":
                    metadata.unlink()
                else:
                    value = json.loads(metadata.read_text())
                    if mutation == "unknown": value["extra"] = True
                    elif mutation == "format": value["format"] = "scientific-workflow-inputs.v0"
                    elif mutation == "checksum": value["config"]["checksum"] = "sha256:abc"
                    else: value["config"]["path"] = "../workflow-config.json"
                    metadata.write_text(json.dumps(value))
                with self.assertRaises(ProjectLayoutError):
                    parameters(snapshot=root / "workflow-config.json")
                with self.assertRaises(DependencyError):
                    Dependencies.load(root / "workflow-dependencies.json")


class ReportingTests(unittest.TestCase):
    def test_opt_in_logging_is_idempotent_and_frames_validate(self):
        import io
        import logging
        from contextlib import redirect_stderr
        from scientific_workflow.reporting import install_logging, progress
        logger = logging.getLogger("workflow.test.reporting")
        logger.setLevel(logging.INFO)
        handler = install_logging(logger)
        try:
            self.assertIs(handler, install_logging(logger))
            with redirect_stderr(io.StringIO()) as output:
                logger.warning("visible")
                progress("conversion", 1, 2, unit="members")
            frames = [json.loads(line.removeprefix("@workflow ")) for line in output.getvalue().splitlines()]
            self.assertEqual([frame["kind"] for frame in frames], ["log", "progress"])
            self.assertEqual(frames[0]["level"], "warning")
            with self.assertRaises(ValueError): progress("invalid", 3, 2)
        finally:
            logger.removeHandler(handler)
            handler.close()
