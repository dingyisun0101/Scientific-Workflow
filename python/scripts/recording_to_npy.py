#!/usr/bin/env python3
"""Source-checkout entry point for the standard Workflow NPY converter."""

from __future__ import annotations

from pathlib import Path
import sys

SOURCE = Path(__file__).resolve().parents[1] / "src"
sys.path.insert(0, str(SOURCE))

from scientific_workflow.npy import main  # noqa: E402


if __name__ == "__main__":
    main()
