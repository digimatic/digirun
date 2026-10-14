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

# GPX only (no TCX), specific output dir
uv run garmin-runs --type trail --format gpx --out ./trails
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
YYYY-MM-DD_HHMM_<name>_<activityId>.gpx   (track/map)
YYYY-MM-DD_HHMM_<name>_<activityId>.tcx   (training data: HR, laps, ...)
```

and `./runs/summary.csv` is (re)written each run with columns:
`date, start_local, activity_id, name, type, distance_km, moving_time,
avg_pace_s_per_km, elevation_gain_m, avg_hr, max_hr, calories`.

Already-downloaded files are skipped, so the tool is safe to re-run — it
only fetches what's missing and refreshes the summary.

## Analysis: walk vs run

`garmin-analyze` works entirely offline on the files in `./runs` — no login, no
network. It splits each activity into time windows, classifies every window as
walk or run, and reports mean pace + standard deviation per class, per activity,
per week, and pooled overall.

```bash
uv run garmin-analyze                                   # ./runs, method=hybrid
uv run garmin-analyze --method pace --pace-threshold 9:00
uv run garmin-analyze --window 30 --min-segment 20 --gap 60
uv run garmin-analyze --start 2026-08-01 --end 2026-09-30 --no-csv
uv run garmin-analyze --cadence-stats            # add cadence mean + stdev
```

### How segments are classified

| Method | Rule |
| --- | --- |
| `hybrid` (default) | cadence < 61 strides/min = walk, > 71 = run; the ambiguous 61–71 band is resolved by the pace threshold |
| `cadence` | cadence ≥ 65 strides/min = run, pace threshold as fallback when cadence is missing |
| `pace` | pace faster than 9:00/km = run |
| `lap` | each TCX lap is one segment, labelled by its own pace |

Cadence is the most reliable signal in this data: it is clearly bimodal (walk
~48–60, run ~72–87, with a gap between 61 and 71), while the pace histogram runs
continuously through 8–10 min/km.

Pace is derived from position deltas (GPX has no speed field) or from TCX
`DistanceMeters`. Pauses longer than `--gap` end a segment, so stopped time is not
counted as walking, and windows shorter than `--min-segment` / `--min-distance` are
ignored as GPS noise.

### Analysis flags

| Flag | Description |
| --- | --- |
| `--out` | directory of downloaded files (default `./runs`) |
| `--start` / `--end` | restrict to a date range |
| `--method` | `hybrid` (default), `cadence`, `pace`, `lap` |
| `--cadence-threshold` | strides/min at or above which a segment is a run (default 65) |
| `--pace-threshold` | pace cutoff as `M:SS` or seconds (default `9:00`) |
| `--window` | seconds of track per segment (default 30) |
| `--min-segment` / `--min-distance` | ignore segments below these (default 20 s / 3 m) |
| `--gap` | a pause longer than this ends the segment (default 60 s) |
| `--max-pace` | exclude segments slower than this from the stats, e.g. `15:00` (default: none) |
| `--cadence-stats` | also report cadence mean + stdev (strides/min) |
| `--report-dir` | where the CSVs are written (default `./runs/analysis`) |
| `--no-csv` | print the report only |

`--cadence-stats` adds `cad`/`sd` columns to the console tables and
`mean_cadence`/`stdev_cadence` to `analysis.csv`. Cadence stdev is over segments,
so it is blank when a class has only one segment.

`--max-pace` is mainly useful for the walk numbers: segments where you stood still
are still classified as walking, and they inflate the stdev. On this data
`--max-pace 15:00` drops 16 of 629 walk segments and brings walk stdev from 1:47
to 1:26. Excluded segments are reported in the header and are left out of both CSVs.

### Analysis output

- `runs/analysis/analysis.csv` — one row per activity × class: `date, activity_id,
  name, class, segments, time_s, distance_km, mean_pace_s_per_km,
  stdev_pace_s_per_km, min_pace_s_per_km, max_pace_s_per_km, share_pct`
- `runs/analysis/segments.csv` — one row per segment: `activity_id, date, time,
  class, pace_s_per_km, cadence, distance_m, window_s`

Example (this data, with `--cadence-stats --max-pace 15:00`):

```
Overall
class       n    time  dist km    mean     sd     min     max  share    cad     sd
run       498 4:27:22    37.18    7:17   0:48    4:53   10:02  45.0%   77.0    4.1
walk      613 5:26:54    31.00   10:44   1:26    5:31   14:57  55.0%   54.7    4.1
```

## Tests

```bash
uv run pytest
```

Pace values in the CSVs are seconds per kilometre, matching `summary.csv`.
