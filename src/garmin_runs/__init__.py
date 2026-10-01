"""garmin-runs: download runs from Garmin Connect."""

from __future__ import annotations

from .cli import main

__all__ = ["main"]


if __name__ == "__main__":
    raise SystemExit(main())
