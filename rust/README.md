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

## Deliberate differences

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
| `src/dt.rs` | timestamps, dates, ISO weeks |
| `src/csv.rs` / `src/csvout.rs` | CSV writing |
| `src/report.rs` | the console tables |
| `src/lib.rs` / `src/main.rs` | `run()` and the thin binary |
