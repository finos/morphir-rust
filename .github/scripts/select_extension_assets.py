#!/usr/bin/env python3
"""Validate downloaded extension bundles and select assets safe to upload."""

from __future__ import annotations

from pathlib import Path
import os
import sys

_SCRIPT_DIRECTORY = os.fspath(Path(__file__).resolve().parent)
if _SCRIPT_DIRECTORY not in sys.path:
    sys.path.insert(0, _SCRIPT_DIRECTORY)

from extension_asset_selection import *


if __name__ == "__main__":
    raise SystemExit(main())
