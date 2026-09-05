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
