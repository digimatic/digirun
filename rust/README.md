# Rust port of `garmin-analyze`

`rust/` is a dependency-light Rust port of `src/garmin_runs/analyze.py`. It reads
the same files in `./runs` and produces the same report and the same CSVs. The
only dependency is `quick-xml`.

```bash
cargo build      # -> rust/target/debug/garmin-analyze
cargo test       # the Python test suite, ported
```

Run it from the repo root, because the default `--out` is `./runs`:

```bash
./rust/target/debug/garmin-analyze --max-pace 15:00 --min-pace 3:00 --cadence-stats
```

## What matches the Python tool

- the report tables, both CSVs, and the `Wrote ...` line
- number formatting: Python's `str(float)` (`714.3`, whole numbers keep `.0`),
  `round(x, 1)` as a correctly rounded decimal (half-even), pace and duration
  formatting, blank cells where Python prints `-`
- CSV dialect: CRLF line endings, `QUOTE_MINIMAL`, the same column order
- ordering: files in sorted order, activities in that same order, segments sorted
  by `(date, time)`, weeks in ISO order
- CLI surface, defaults, and exit codes (0 for help, 1 for program errors such as
  a bad pace or a missing directory, 2 for argument errors)
- `--help` and argument errors are laid out the way argparse does, including the
  usage wrapping and the option column width

## Detail mode (Rust only)

`--detail` lists every run and walk **pass** (one continuous stretch of running
or walking) in each session, with its stats. With no `--start/--end/--activity`
it covers the last 7 days, today included:

```bash
./rust/target/release/garmin-analyze --max-pace 15:00 --min-pace 3:00 --detail
./rust/target/release/garmin-analyze --detail --activity 12345678901
```

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
Passes outside `--max-pace/--min-pace` are marked `*` and left out of the totals.
`passes.csv` in `--report-dir` holds one row per pass.

## Deliberate differences

- `--detail` (above) exists only here.
- A file that fails to parse is reported on stderr and skipped. The Python tool
  raises and stops the whole run, so one corrupt file takes down the report.
- `--jobs` uses threads instead of processes. The pool is skipped below 8 files,
  and results keep the sorted file order, so the output is the same as `--jobs 1`.
- Help wrapping uses `$COLUMNS` when it is set and otherwise 80 columns. argparse
  reads the terminal size directly, so interactive help can wrap differently.

## Speed

Same flags on the 255 downloaded activities, best of three runs:

| | Python | Rust |
| --- | --- | --- |
| full report, 8 workers | 1.2 s | 0.16 s |
| single worker (`--jobs 1`) | 3.2 s | 0.60 s |

The Python figures include `uv` startup. Threads also scale better than the process
pool: 3.75x from one worker to eight, against 2.7x for Python.

## Layout

| File | Contents |
| --- | --- |
| `src/args.rs` | argument parsing, defaults, argparse-style help and errors |
| `src/load.rs` | grouping the files per activity, reading `summary.csv`, the worker pool |
| `src/parse.rs` | `read_gpx` / `read_tcx` |
| `src/xml.rs` | small namespace-agnostic DOM, so `_find`/`_text` behave as in Python |
| `src/model.rs` | `Point`, `Activity`, `Segment`, haversine |
| `src/analyze.rs` | classification, segmentation, stats, filters, `round` |
| `src/passes.rs` | `--detail`: cadence-split passes and their stats |
| `src/dt.rs` | timestamps, dates, ISO weeks |
| `src/csv.rs` / `src/csvout.rs` | CSV writing |
| `src/report.rs` | the console tables |
| `src/lib.rs` / `src/main.rs` | `run()` and the thin binary |
