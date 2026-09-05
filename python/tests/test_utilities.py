import json
import tempfile
import unittest
from pathlib import Path
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
            self.assertEqual(parameters("analysis", snapshot=snapshot), {"bins": 3})
            with self.assertRaisesRegex(ProjectLayoutError, "wf_configs/study.json"): study_path(root)
            with self.assertRaisesRegex(ProjectLayoutError, "missing"): parameters("missing", snapshot=snapshot)

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
