"""Focused accessors for the REQUIRED standard study layout and launch environment.

These helpers do not discover relocated files, activate environments, change
working directories, or create output. Program snapshots contain resolved values.
"""
import json
import os
from pathlib import Path


class ProjectLayoutError(ValueError):
    """A required standard path or snapshot is absent or malformed."""


def _environment_path(variable: str, *, directory: bool) -> Path:
    value = os.environ.get(variable)
    if not value:
        raise ProjectLayoutError(f"missing {variable}; run through Workflow's REQUIRED standard study layout")
    path = Path(value)
    if not path.is_absolute() or not (path.is_dir() if directory else path.is_file()):
        raise ProjectLayoutError(f"expected {'directory' if directory else 'file'} at {path} from {variable}; preserve Workflow's REQUIRED standard layout")
    return path


def project_root() -> Path:
    """Return the launched program's WORKFLOW_PROJECT_ROOT."""
    return _environment_path("WORKFLOW_PROJECT_ROOT", directory=True)


def output_directory() -> Path:
    """Return the current program's writable standard artifacts directory."""
    return _environment_path("WORKFLOW_TASK_OUTPUT", directory=True)


def study_path(root: str | Path) -> Path:
    """Require <root>/wf_configs/study.json, without parsing source configuration."""
    path = Path(root) / "wf_configs" / "study.json"
    if not path.is_file():
        raise ProjectLayoutError(f"expected {path}; REQUIRED layout: <study>/wf_configs/study.json")
    return path


def parameters(section: str | None = None, *, snapshot: str | Path | None = None) -> object:
    """Read resolved parameters, optionally selecting one exact top-level key.

    Omit snapshot only inside a Workflow-launched program. This does not read
    unresolved wf_configs/parameters.json or implement a second resolution graph.
    """
    path = Path(snapshot) if snapshot is not None else _environment_path("WORKFLOW_CONFIG_PATH", directory=False)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))["config"]["parameters.json"]
        if not isinstance(value, dict):
            raise ValueError("parameters.json must contain an object")
        return value if section is None else value[section]
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise ProjectLayoutError(f"cannot read resolved parameters {section!r} at {path}; expected Workflow's standard workflow-config.json snapshot: {error}") from error
