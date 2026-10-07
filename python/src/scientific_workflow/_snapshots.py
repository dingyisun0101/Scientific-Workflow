"""Private verification of task-owned configuration and dependency snapshots."""

import hashlib
from pathlib import Path
import re

from ._json import decode

_FORMAT = "scientific-workflow-inputs.v1"
_FILES = {"config": "workflow-config.json", "dependencies": "workflow-dependencies.json"}
_CHECKSUM = re.compile(r"sha256:[0-9a-f]{64}\Z")


def read(path: Path, kind: str):
    """Verify both captured files before decoding the selected snapshot."""
    root = path.parent
    metadata_path = root / "workflow-inputs.json"
    metadata = decode(metadata_path.read_bytes())
    if (
        not isinstance(metadata, dict)
        or set(metadata) != {"format", *_FILES}
        or metadata["format"] != _FORMAT
    ):
        raise ValueError(f"unsupported task input metadata at {metadata_path}")
    refs = {key: metadata[key] for key in _FILES}
    for key, filename in _FILES.items():
        ref = refs[key]
        if (
            not isinstance(ref, dict)
            or set(ref) != {"path", "checksum"}
            or ref["path"] != filename
            or not isinstance(ref["checksum"], str)
            or _CHECKSUM.fullmatch(ref["checksum"]) is None
        ):
            raise ValueError(f"invalid {key} snapshot descriptor at {metadata_path}")
    if path.name != _FILES[kind]:
        raise ValueError(f"expected task-owned {_FILES[kind]} at {path}")
    for filename, expected_format in (
        ("program.json", "scientific-workflow-program-v2"),
        ("workflow-result.json", "scientific-workflow-task-result.v2"),
    ):
        authority_path = root / filename
        if authority_path.exists():
            authority = decode(authority_path.read_bytes())
            if (
                not isinstance(authority, dict)
                or authority.get("format") != expected_format
                or authority.get("inputs") != refs
            ):
                raise ValueError(f"task input descriptors differ from current metadata at {authority_path}")
    snapshots = {}
    for key, filename in _FILES.items():
        snapshot_path = root / filename
        data = snapshot_path.read_bytes()
        if "sha256:" + hashlib.sha256(data).hexdigest() != refs[key]["checksum"]:
            raise ValueError(f"captured input checksum mismatch at {snapshot_path}")
        snapshots[key] = data
    return decode(snapshots[kind])
