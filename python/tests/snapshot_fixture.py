"""Test-only captured-input evidence; never used as a package writer API."""

import hashlib
import json
from pathlib import Path


def seal_inputs(root: Path) -> dict:
    files = {"config": "workflow-config.json", "dependencies": "workflow-dependencies.json"}
    defaults = {"config": {"config": {"parameters.json": {}}}, "dependencies": []}
    refs = {}
    for kind, name in files.items():
        path = root / name
        if not path.exists():
            path.write_text(json.dumps(defaults[kind]))
        refs[kind] = {"path": name, "checksum": "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()}
    (root / "workflow-inputs.json").write_text(json.dumps({
        "format": "scientific-workflow-inputs.v1", **refs,
    }))
    return refs
