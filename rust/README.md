# garmin-analyze

The walk vs run analyzer. It reads the files `garmin-runs` downloads into
`./runs` and works offline. The only dependency is `quick-xml`; the flags and
the classification are described in the top-level `README.md`.

```bash
cargo build --release    # -> rust/target/release/garmin-analyze
cargo test
```

Run it from the repo root, because the default `--out` is `./runs`:

```bash
./rust/target/release/garmin-analyze --max-pace 15:00 --min-pace 3:00 --cadence-stats
```

## Tables and range

`--show` picks the tables: `activities`, `weekly`, `overall` (the default is these
three), `detail`, or `all`. They all cover the same range, and every table is
narrowed by `--start/--end`, `--days N` (the last N days up to `--end` or today), and
`--activity ID[,ID]`. Without a range, the tables cover all of time.

```bash
./rust/target/release/garmin-analyze --max-pace 15:00 --min-pace 3:00 --show detail --days 7
./rust/target/release/garmin-analyze --show weekly,detail --start 2026-10-01
./rust/target/release/garmin-analyze --show detail --activity 12345678901
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
- `--jobs` parses on a pool of threads. The pool is skipped below 8 files, and
  results keep the sorted file order, so the output is the same as `--jobs 1`.
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
