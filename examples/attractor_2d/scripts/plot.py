"""Render all completed Hopf trajectories from Workflow dependency outputs."""

from __future__ import annotations

import html
import importlib
import json
import os
import sys
from pathlib import Path
from typing import Any


def _workflow_reader() -> Any:
    """Import the installed reader, with a source-checkout fallback for this example."""

    try:
        return importlib.import_module("scientific_workflow_reader")
    except ModuleNotFoundError:
        project_root = Path(os.environ["WORKFLOW_PROJECT_ROOT"])
        checkout_source = project_root.parents[1] / "python" / "src"
        if not checkout_source.is_dir():
            raise
        sys.path.insert(0, str(checkout_source))
        return importlib.import_module("scientific_workflow_reader")


def _object(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be a JSON object")
    return value


def _string(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be a nonempty string")
    return value


def _number(value: Any, name: str, *, positive: bool = False) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be numeric")
    number = float(value)
    if positive and number <= 0.0:
        raise ValueError(f"{name} must be positive")
    return number


def _settings() -> dict[str, Any]:
    snapshot = json.loads(Path(os.environ["WORKFLOW_CONFIG_PATH"]).read_text())
    config = _object(snapshot, "configuration snapshot").get("config")
    documents = _object(config, "configuration snapshot.config")
    parameters = _object(
        documents.get("parameters.json"), "config/parameters.json"
    )
    return _object(parameters.get("plot"), "config/parameters.json.plot")


def _output_directory(settings: dict[str, Any]) -> Path:
    authored = Path(
        _string(settings.get("output_directory"), "plot.output_directory")
    )
    if authored.is_absolute() or not authored.parts or any(
        part in {"", ".", ".."} for part in authored.parts
    ):
        raise ValueError(
            "plot.output_directory must be a normalized project-relative path"
        )
    project_root = Path(os.environ["WORKFLOW_PROJECT_ROOT"]).resolve(strict=True)
    output = (project_root / authored).resolve()
    if not output.is_relative_to(project_root):
        raise ValueError("plot.output_directory escapes the project root")
    output.mkdir(parents=True, exist_ok=True)
    return output


def _recordings() -> list[Path]:
    dependencies = json.loads(
        Path(os.environ["WORKFLOW_DEPENDENCIES_PATH"]).read_text()
    )
    if not isinstance(dependencies, list):
        raise ValueError("dependency snapshot must be an array")
    recordings: list[Path] = []
    for phase in dependencies:
        phase = _object(phase, "dependency phase")
        if phase.get("phase") != "simulate":
            continue
        tasks = phase.get("tasks")
        if not isinstance(tasks, list):
            raise ValueError("dependency phase tasks must be an array")
        for task in tasks:
            task = _object(task, "dependency task")
            if task.get("kind") != "model":
                continue
            recordings.append(Path(_string(task.get("output_directory"), "task output")))
    if not recordings:
        raise ValueError("plot phase received no simulation recordings")
    return recordings


def _trajectories(stream: str) -> list[dict[str, Any]]:
    reader_module = _workflow_reader()
    trajectories: list[dict[str, Any]] = []
    for recording in _recordings():
        reader = reader_module.open_completed_recording(recording)
        series = reader.read_stream(stream)
        points: list[tuple[float, float]] = []
        for record in series:
            point = record.values.get("point")
            if (
                not isinstance(point, list)
                or len(point) != 2
                or any(isinstance(value, bool) or not isinstance(value, (int, float)) for value in point)
            ):
                raise ValueError(f"recording {recording} contains an invalid point payload")
            points.append((float(point[0]), float(point[1])))
        if not points:
            raise ValueError(f"recording {recording} has an empty {stream!r} stream")
        constants = _object(reader.user_metadata.get("model_constants"), "model constants")
        trajectories.append(
            {
                "recording": str(recording),
                "mu": _number(constants.get("mu"), "model_constants.mu"),
                "omega": _number(
                    constants.get("angular_frequency"),
                    "model_constants.angular_frequency",
                ),
                "points": points,
            }
        )
    trajectories.sort(key=lambda item: (item["mu"], item["omega"]))
    return trajectories


def _svg(settings: dict[str, Any], trajectories: list[dict[str, Any]]) -> str:
    width = int(_number(settings.get("width"), "plot.width", positive=True))
    height = int(_number(settings.get("height"), "plot.height", positive=True))
    margin = _number(settings.get("margin"), "plot.margin", positive=True)
    stroke_width = _number(
        settings.get("stroke_width"), "plot.stroke_width", positive=True
    )
    if width <= 2 * margin or height <= 2 * margin:
        raise ValueError("plot dimensions must exceed twice the margin")
    palette = settings.get("palette")
    if not isinstance(palette, list) or not palette:
        raise ValueError("plot.palette must be a nonempty array")
    colors = [_string(color, "plot.palette item") for color in palette]
    title = html.escape(_string(settings.get("title"), "plot.title"))
    background = html.escape(_string(settings.get("background"), "plot.background"))
    foreground = html.escape(_string(settings.get("foreground"), "plot.foreground"))
    axis = html.escape(_string(settings.get("axis"), "plot.axis"))

    all_points = [point for trajectory in trajectories for point in trajectory["points"]]
    extent = max(max(abs(x), abs(y)) for x, y in all_points)
    extent = max(extent * 1.08, 1.0e-12)
    plot_width = width - 2.0 * margin
    plot_height = height - 2.0 * margin

    def project(point: tuple[float, float]) -> tuple[float, float]:
        x, y = point
        return (
            margin + (x / extent + 1.0) * plot_width / 2.0,
            margin + (1.0 - y / extent) * plot_height / 2.0,
        )

    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        f'<rect width="100%" height="100%" fill="{background}"/>',
        f'<text x="{width / 2:.1f}" y="36" text-anchor="middle" fill="{foreground}" font-family="sans-serif" font-size="22">{title}</text>',
        f'<line x1="{margin:.1f}" y1="{height / 2:.1f}" x2="{width - margin:.1f}" y2="{height / 2:.1f}" stroke="{axis}"/>',
        f'<line x1="{width / 2:.1f}" y1="{margin:.1f}" x2="{width / 2:.1f}" y2="{height - margin:.1f}" stroke="{axis}"/>',
    ]
    for index, trajectory in enumerate(trajectories):
        color = html.escape(colors[index % len(colors)])
        points = " ".join(
            f"{x:.2f},{y:.2f}" for x, y in map(project, trajectory["points"])
        )
        lines.append(
            f'<polyline points="{points}" fill="none" stroke="{color}" stroke-width="{stroke_width:.2f}" stroke-linejoin="round" stroke-linecap="round"/>'
        )
        legend_y = 58 + index * 20
        label = html.escape(
            f'mu={trajectory["mu"]:g}, omega={trajectory["omega"]:g}'
        )
        lines.extend(
            [
                f'<line x1="{width - 250}" y1="{legend_y}" x2="{width - 220}" y2="{legend_y}" stroke="{color}" stroke-width="3"/>',
                f'<text x="{width - 212}" y="{legend_y + 4}" fill="{foreground}" font-family="monospace" font-size="13">{label}</text>',
            ]
        )
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


def main() -> None:
    settings = _settings()
    stream = _string(settings.get("stream"), "plot.stream")
    trajectories = _trajectories(stream)
    output = _output_directory(settings)
    output_file = _string(settings.get("output_file"), "plot.output_file")
    if Path(output_file).name != output_file or not output_file.endswith(".svg"):
        raise ValueError("plot.output_file must be a plain `.svg` filename")
    (output / output_file).write_text(_svg(settings, trajectories), encoding="utf-8")
    summary = {
        "format": "scientific-workflow-attractor-plot-v1",
        "stream": stream,
        "output": output_file,
        "trajectories": [
            {
                "recording": trajectory["recording"],
                "mu": trajectory["mu"],
                "angular_frequency": trajectory["omega"],
                "record_count": len(trajectory["points"]),
            }
            for trajectory in trajectories
        ],
    }
    (output / "plot-summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
