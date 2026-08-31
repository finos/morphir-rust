#!/usr/bin/env python3
"""Build a deterministic release bundle for a registered WASM extension."""

import sys

try:
    from extension_packaging.cli import main
except ModuleNotFoundError as error:
    print(f"error: packaging module unavailable: {error}", file=sys.stderr)
    raise SystemExit(1) from None


if __name__ == "__main__":
    raise SystemExit(main())
