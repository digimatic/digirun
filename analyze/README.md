# garmin-analyze

Split the activities `garmin-runs` downloads into walk and run segments. It works
entirely offline on the files in `./runs` (no login, no network): it splits each
activity into time windows, classifies every window as walk or run, and reports
pace (total time / total distance) and the standard deviation of the segment paces
per class, per activity, per week, and pooled overall. The only dependency is
`quick-xml`.

```bash
cargo build --release --manifest-path analyze/Cargo.toml
cargo test --manifest-path analyze/Cargo.toml
```

Run it from the repo root, because the default `--out` is `./runs`:

```bash
alias garmin-analyze=./analyze/target/release/garmin-analyze

garmin-analyze                                   # ./runs, method=hybrid
garmin-analyze --method pace --pace-threshold 9:00
garmin-analyze --window 30 --min-segment 20 --gap 60
garmin-analyze --start 2026-08-01 --end 2026-09-30 --no-csv
garmin-analyze --cadence-stats            # add cadence mean + stdev
garmin-analyze --max-pace 15:00 --min-pace 3:00 --cadence-stats   # the usual set
```

## How segments are classified

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

## Flags

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

## Output

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

Pace values in the CSVs are seconds per kilometre, matching `summary.csv`.

## Tables and range

`--show` picks the tables: `activities`, `weekly`, `overall` (the default is these
three), `detail`, or `all`. They all cover the same range, and every table is
narrowed by `--start/--end`, `--days N` (the last N days up to `--end` or today), and
`--activity ID[,ID]`. Without a range, the tables cover all of time.

```bash
garmin-analyze --max-pace 15:00 --min-pace 3:00 --show detail --days 7
garmin-analyze --show weekly,detail --start 2026-10-01
garmin-analyze --show detail --activity 12345678901
```

## The detail table

`detail` lists every run and walk **pass** (one continuous stretch of running or
walking) in each session, with its stats.

Passes are split on cadence, not on Garmin's laps, because a lap (a workout step
or a 1 km auto-split) can hold both running and walking:

1. cadence is smoothed with a rolling median over `--smooth` seconds (default 10)
2. each trackpoint interval is labelled with hysteresis: above 71 strides/min is
   a run, below 61 a walk, and inside the band the previous label holds. With
   `--method cadence`, `--cadence-threshold` is a single cut-off instead. An
   activity without cadence, or `--method pace`, uses a rolling pace against
   `--pace-threshold`
3. consecutive intervals with the same label make a pass, and a pause longer than
   `--gap` ends it
4. passes shorter than `--min-pass` seconds (default 20) join their longer neighbour

Each pass shows its start, time, distance, pace (with sd/min/max over the
`--window` sub-windows), cadence mean/sd/min/max, HR mean/min/max, HR drift (last
third minus first third), elevation gain/loss, stride length, and the Garmin laps
it overlaps. Each session gets per-class totals, and the whole range gets a summary.
Within a pass, `--window` sub-windows outside `--max-pace/--min-pace` (standing
about) are left out of every stat, and the `stop` column shows how long they were.
Passes that are outside the guard as a whole are marked `*` and left out of the totals.
`passes.csv` in `--report-dir` holds one row per pass.

## Behaviour notes

- The mean pace in the tables and in `analysis.csv` (`mean_pace_s_per_km`) is
  total time / total distance, as Garmin reports it. The sd, min and max are over
  the segment paces.
- A file that fails to parse is reported on stderr and skipped, so one corrupt
  file does not take down the report.
- Numbers, CSVs (CRLF, minimal quoting) and `--help` are formatted the way
  Python's `str(float)`, `csv` module and argparse do. Help wraps at `$COLUMNS`
  when it is set and otherwise at 80 columns.
- Exit codes: 0 for help, 1 for program errors such as a bad pace or a missing
  directory, 2 for argument errors.

## Layout

| File | Contents |
| --- | --- |
| `src/args.rs` | argument parsing, defaults, argparse-style help and errors |
| `src/load.rs` | grouping the files per activity, reading `summary.csv`, the worker pool |
| `src/parse.rs` | `read_gpx` / `read_tcx` |
| `src/xml.rs` | small namespace-agnostic DOM |
| `src/model.rs` | `Point`, `Activity`, `Segment`, haversine |
| `src/analyze.rs` | classification, segmentation, stats, filters, `round` |
| `src/passes.rs` | the detail table: cadence-split passes and their stats |
| `src/dt.rs` | timestamps, dates, ISO weeks |
| `src/csv.rs` / `src/csvout.rs` | CSV writing |
| `src/report.rs` | the console tables |
| `src/lib.rs` / `src/main.rs` | `run()` and the thin binary |
