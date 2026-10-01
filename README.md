# garmin-runs

Download your runs from **Garmin Connect** as GPX files, plus a `summary.csv`
with per-run stats. Built with [`garminconnect`](https://pypi.org/project/garminconnect/)
and managed with [uv](https://docs.astral.sh/uv/).

## Setup

```bash
uv sync
```

## Credentials

The tool never stores your password in the repo. Provide it via environment
variables (recommended) or it will prompt you:

```bash
export GARMIN_EMAIL=you@example.com
export GARMIN_PASSWORD=...          # or leave unset to be prompted
# optional: export GARMIN_IS_CN=true   # for garmin.cn accounts
```

On the first run you are authenticated and session tokens are cached in
`~/.garminconnect`, so you only enter your password (and any MFA code) once.
Later runs reuse the cached tokens.

## Usage

```bash
# Last 90 days of runs (default) -> ./runs/
uv run garmin-runs

# Last 30 days
uv run garmin-runs --days 30

# Explicit range
uv run garmin-runs --start 2026-08-01 --end 2026-08-31

# Specific type + extra formats
uv run garmin-runs --type trail --format gpx,tcx --out ./trails
```

### Flags

| Flag | Description |
| --- | --- |
| `--days N` | Download the last N days |
| `--start` / `--end` | Explicit inclusive range (`YYYY-MM-DD`) |
| `--type` | `runs` (running+trail, default), `all`, or a single type (`running`, `trail`, `cycling`, `swimming`, …) |
| `--format` | Comma-separated: `gpx,tcx,kml,csv,original` (default `gpx`) |
| `--out` | Output directory (default `./runs`) |
| `--tokenstore` | Token cache path (default `~/.garminconnect` or `$GARMINTOKENS`) |
| `--cn` | Use Garmin China (garmin.cn) endpoints |

## Output

Files are written to `./runs/` as:

```
YYYY-MM-DD_HHMM_<name>_<activityId>.gpx
```

and `./runs/summary.csv` is (re)written each run with columns:
`date, start_local, activity_id, name, type, distance_km, moving_time,
avg_pace_s_per_km, elevation_gain_m, avg_hr, max_hr, calories`.

Already-downloaded files are skipped, so the tool is safe to re-run — it
only fetches what's missing and refreshes the summary.
