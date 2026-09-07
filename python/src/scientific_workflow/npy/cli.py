from __future__ import annotations

import argparse
import os
from pathlib import Path

from .workflow import convert_recording, convert_workflow_dependencies

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Convert completed Scientific Workflow recordings to NPY datasets."
    )
    parser.add_argument("recording", type=Path, nargs="?")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--exclude-stream", action="append", default=[], metavar="NAME", help="exclude an exact stream name; repeat for multiple streams")
    parser.add_argument("--workflow-dependencies", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.workflow_dependencies:
        if arguments.recording is not None or arguments.output is not None:
            parser.error("--workflow-dependencies does not accept recording or --output")
        try:
            dependencies_path = os.environ["WORKFLOW_DEPENDENCIES_PATH"]
            output_directory = os.environ["WORKFLOW_NPY_OUTPUT"]
        except KeyError as error:
            parser.error(f"missing required Workflow environment variable {error.args[0]}")
        convert_workflow_dependencies(dependencies_path, output_directory, exclude_streams=arguments.exclude_stream)
        return
    if arguments.recording is None:
        parser.error("recording is required")
    manifest = convert_recording(arguments.recording, arguments.output, exclude_streams=arguments.exclude_stream)
    print(
        f"converted {manifest['source_recording']} into "
        f"{arguments.output or Path(arguments.recording).with_name(Path(arguments.recording).name + '-npy')}",
        flush=True,
    )


if __name__ == "__main__":
    main()
