#!/usr/bin/env python3
"""Classify which CI jobs a change set needs, from the cargo dependency graph."""

from __future__ import annotations

from pathlib import Path
import os
import sys

_SCRIPT_DIRECTORY = os.fspath(Path(__file__).resolve().parent)
if _SCRIPT_DIRECTORY not in sys.path:
    sys.path.insert(0, _SCRIPT_DIRECTORY)

from ci_impact.cli import main


if __name__ == "__main__":
    raise SystemExit(main())
