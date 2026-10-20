# digirun

Download your runs from **Garmin Connect** as GPX/TCX files, plus a `summary.csv`
with per-run stats, then analyze walk vs run pace offline.

- `garmin-runs` (Python, in `src/`) downloads. Built with
  [`garminconnect`](https://pypi.org/project/garminconnect/), which handles the
  Garmin login, and managed with [uv](https://docs.astral.sh/uv/).
- `garmin-analyze` (Rust, in `rust/`) analyzes the downloaded files.

`./analyze.sh` does both: it fetches at most once an hour, then prints the report
and the run/walk passes of the last 7 days.

## Setup

```bash
uv sync                                # the downloader
cd rust && cargo build --release       # the analyzer
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
walk or run, and reports pace (total time / total distance) and the standard
deviation of the segment paces per class, per activity, per week, and pooled
overall.

Run it from the repo root, because the default `--out` is `./runs`. See
`rust/README.md` for the `--show` tables, the detail table, and the code layout.

```bash
alias garmin-analyze=./rust/target/release/garmin-analyze

garmin-analyze                                   # ./runs, method=hybrid
garmin-analyze --method pace --pace-threshold 9:00
garmin-analyze --window 30 --min-segment 20 --gap 60
garmin-analyze --start 2026-08-01 --end 2026-09-30 --no-csv
garmin-analyze --cadence-stats            # add cadence mean + stdev
garmin-analyze --max-pace 15:00 --min-pace 3:00 --cadence-stats   # the usual set
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
| `--type` | activity types to analyze: `all`, or comma-separated keys (default `running,trail,walking`) |
| `--days N` | only the last N days, up to `--end` or today |
| `--activity` | only these comma-separated activity ids |
| `--jobs` | worker threads used to parse the files (default `min(cpu_count, 8)`; `1` runs single-threaded) |
| `--method` | `hybrid` (default), `cadence`, `pace`, `lap` |
| `--cadence-threshold` | strides/min at or above which a segment is a run (default 65) |
| `--pace-threshold` | pace cutoff as `M:SS` or seconds (default `9:00`) |
| `--window` | seconds of track per segment (default 30) |
| `--min-segment` / `--min-distance` | ignore segments below these (default 20 s / 3 m) |
| `--gap` | a pause longer than this ends the segment (default 60 s) |
| `--max-pace` | exclude segments slower than this from the stats, e.g. `15:00` (default: none) |
| `--min-pace` | exclude segments faster than this from the stats, e.g. `3:00` (default: none) |
| `--cadence-stats` | also report cadence mean + stdev (strides/min) |
| `--min-pass` / `--smooth` | detail table: shortest pass, and cadence smoothing (default 20 s / 10 s) |
| `--show` | tables to print: `activities,weekly,overall` (default), `detail`, or `all` |
| `--report-dir` | where the CSVs are written (default `./runs/analysis`) |
| `--no-csv` | print the report only |

`--cadence-stats` adds `cad`/`sd` columns to the console tables and
`mean_cadence`/`stdev_cadence` to `analysis.csv`. Cadence stdev is over segments,
so it is blank when a class has only one segment.

`--max-pace` and `--min-pace` guard the stats against segments that are not really
movement. `--max-pace 15:00` drops the standing-about segments that are still
classified as walking and inflate the walk stdev; `--min-pace 3:00` drops segments
where the track shows 400–500 m covered in a 30 s window (50+ km/h), which is GPS
drift or being carried rather than running. Excluded segments are reported in the
header and are left out of both CSVs.

`--type` keeps other sports out of the stats. The GPX/TCX files do not record the
activity type, so it is read from `runs/summary.csv`; activities missing from that
file are skipped and reported as unknown type. Without the filter, cycling segments
(2–3 min/km) are classified as run and lift or coast time as walk, which skews both
numbers — so `--type all` is fine for archiving, but analyze it with a filter.

`--jobs` parses the files on a pool of threads. Results keep the sorted file order,
so the report and the CSVs are the same with `--jobs 1` and `--jobs 8`; below 8
activities the pool is skipped. The 255 activities here take 0.16 s with 8 threads.

### Analysis output

- `runs/analysis/analysis.csv` — one row per activity × class: `date, activity_id,
  name, type, class, segments, time_s, distance_km, mean_pace_s_per_km,
  stdev_pace_s_per_km, min_pace_s_per_km, max_pace_s_per_km, share_pct`
  (plus `mean_cadence, stdev_cadence` with `--cadence-stats`)
- `runs/analysis/segments.csv` — one row per segment: `activity_id, date, time,
  class, pace_s_per_km, cadence, distance_m, window_s`
- `runs/analysis/passes.csv` — one row per run/walk pass, with `--show detail`

Example (this data, with `--max-pace 15:00 --min-pace 3:00 --cadence-stats`):

```
Overall
class       n    time  dist km    mean     sd     min     max  share    cad     sd
----------------------------------------------------------------------------------
run      1965 17:37:22   144.55    7:19   0:57    3:21   12:18  26.8%   76.8    3.7
walk     5356 48:11:23   252.50   11:27   1:28    5:24   15:00  73.2%   53.4    5.7
```

## Tests

```bash
cd rust && cargo test
```

Pace values in the CSVs are seconds per kilometre, matching `summary.csv`.
