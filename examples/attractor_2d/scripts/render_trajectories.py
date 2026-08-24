#!/usr/bin/env python3
"""Render every completed attractor trajectory in one study execution."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

# Use the reader from this checkout; the DSES environment supplies matplotlib.
WORKFLOW_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKFLOW_ROOT / "python" / "src"))

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

from scientific_workflow_reader import open_completed_recording


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--recording-directory", required=True, type=Path)
    parser.add_argument("--output-directory", required=True, type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    recordings = sorted(args.recording_directory.glob("task-[0-9][0-9][0-9][0-9][0-9][0-9]"))
    if not recordings:
        raise RuntimeError(f"no task recordings found in {args.recording_directory}")

    args.output_directory.mkdir(parents=True, exist_ok=True)
    for recording in recordings:
        reader = open_completed_recording(recording)
        trajectory = reader.read_stream("trajectory")
        points = [state.values["point"] for state in trajectory]
        x = [point[0] for point in points]
        y = [point[1] for point in points]
        mu = reader.user_metadata["mu"]
        omega = reader.user_metadata["angular_frequency"]
        ordinal = int(reader.user_metadata["ordinal"])

        figure, axes = plt.subplots(figsize=(6, 6), constrained_layout=True)
        axes.plot(x, y, linewidth=1.4)
        axes.scatter([x[0]], [y[0]], label="start", s=28)
        axes.scatter([x[-1]], [y[-1]], label="end", s=28)
        axes.set(title=f"mu={mu}, omega={omega}", xlabel="x", ylabel="y")
        axes.set_aspect("equal", adjustable="box")
        axes.grid(alpha=0.25)
        axes.legend()
        figure.savefig(args.output_directory / f"trajectory-{ordinal:06}.png", dpi=160)
        plt.close(figure)

    print(f"rendered {len(recordings)} trajectories to {args.output_directory}")


if __name__ == "__main__":
    main()
