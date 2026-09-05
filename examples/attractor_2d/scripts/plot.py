"""Plot Hopf trajectories from Workflow's standard processed NPY output."""

from __future__ import annotations

import html
import json
import os
from pathlib import Path
from typing import Any

import numpy as np
from scientific_workflow.npy import open_npy_batch


def _json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object at {path}")
    return value


def _settings() -> dict[str, Any]:
    snapshot = _json(Path(os.environ["WORKFLOW_CONFIG_PATH"]))
    return snapshot["config"]["parameters.json"]["plot"]


def _processed_root() -> Path:
    dependencies = json.loads(
        Path(os.environ["WORKFLOW_DEPENDENCIES_PATH"]).read_text(encoding="utf-8")
    )
    npy_phase = next(phase for phase in dependencies if phase["phase"] == "$npy")
    npy_task = next(
        task for task in npy_phase["tasks"] if task["workload"]["kind"] == "npy"
    )
    return Path(npy_task["workload"]["processed_directory"])


def _trajectories(root: Path, stream: str) -> list[dict[str, Any]]:
    batch = open_npy_batch(root)
    trajectories = []
    for member in batch.members:
        field = member.field(stream, "point")
        dataset = field["dataset"]
        if dataset["storage"] != "fixed":
            raise ValueError("point field must have fixed numeric storage")
        points = member.array(dataset["data"])
        if points.ndim != 2 or points.shape[1] != 2 or not points.flags.c_contiguous:
            raise ValueError("point array must be C-contiguous with shape (N, 2)")
        constants = member.manifest["user_metadata"]["constants"]
        trajectories.append(
            {
                "mu": float(constants["mu"]),
                "omega": float(constants["angular_frequency"]),
                "points": points,
                "manifest": str(member.directory / "manifest.json"),
            }
        )
    return sorted(trajectories, key=lambda item: (item["mu"], item["omega"]))


def _svg(settings: dict[str, Any], trajectories: list[dict[str, Any]]) -> str:
    width, height = int(settings["width"]), int(settings["height"])
    margin = float(settings["margin"])
    extent = max(
        1.0e-12,
        max(
            max(abs(float(point[0])), abs(float(point[1])))
            for trajectory in trajectories
            for point in trajectory["points"]
        )
        * 1.08,
    )

    def project(point: np.ndarray[Any, Any]) -> tuple[float, float]:
        return (
            margin + (float(point[0]) / extent + 1.0) * (width - 2 * margin) / 2,
            margin + (1.0 - float(point[1]) / extent) * (height - 2 * margin) / 2,
        )

    foreground = html.escape(settings["foreground"])
    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        f'<rect width="100%" height="100%" fill="{html.escape(settings["background"])}"/>',
        f'<text x="{width / 2:.1f}" y="36" text-anchor="middle" fill="{foreground}" font-family="sans-serif" font-size="22">{html.escape(settings["title"])}</text>',
        f'<line x1="{margin}" y1="{height / 2}" x2="{width - margin}" y2="{height / 2}" stroke="{html.escape(settings["axis"])}"/>',
        f'<line x1="{width / 2}" y1="{margin}" x2="{width / 2}" y2="{height - margin}" stroke="{html.escape(settings["axis"])}"/>',
    ]
    for index, trajectory in enumerate(trajectories):
        color = html.escape(settings["palette"][index % len(settings["palette"])])
        points = " ".join(
            f"{x:.2f},{y:.2f}" for x, y in map(project, trajectory["points"])
        )
        legend_y = 58 + index * 20
        label = html.escape(f'mu={trajectory["mu"]:g}, omega={trajectory["omega"]:g}')
        lines.extend(
            [
                f'<polyline points="{points}" fill="none" stroke="{color}" stroke-width="{float(settings["stroke_width"]):.2f}"/>',
                f'<line x1="{width - 250}" y1="{legend_y}" x2="{width - 220}" y2="{legend_y}" stroke="{color}" stroke-width="3"/>',
                f'<text x="{width - 212}" y="{legend_y + 4}" fill="{foreground}" font-family="monospace" font-size="13">{label}</text>',
            ]
        )
    return "\n".join([*lines, "</svg>", ""])


def main() -> None:
    settings = _settings()
    processed_root = _processed_root()
    trajectories = _trajectories(processed_root, settings["stream"])
    output = Path(os.environ["WORKFLOW_PROJECT_ROOT"]) / settings["output_directory"]
    output.mkdir(parents=True, exist_ok=True)
    (output / settings["output_file"]).write_text(
        _svg(settings, trajectories), encoding="utf-8"
    )
    summary = {
        "format": "scientific-workflow-attractor-plot-v3",
        "processed_data": str(processed_root),
        "trajectories": [
            {
                "mu": trajectory["mu"],
                "angular_frequency": trajectory["omega"],
                "record_count": len(trajectory["points"]),
                "manifest": trajectory["manifest"],
            }
            for trajectory in trajectories
        ],
    }
    (output / "plot-summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
