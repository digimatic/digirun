# digirun

Download your runs from **Garmin Connect**, then analyze walk vs run pace offline.

| Directory | Tool | |
| --- | --- | --- |
| [`fetch/`](fetch/README.md) | `garmin-runs` (Python) | downloads activities as GPX/TCX plus a `summary.csv` |
| [`analyze/`](analyze/README.md) | `garmin-analyze` (Rust) | splits them into walk and run, and reports pace and cadence |

Both run from the repo root and share `./runs` as the data directory, which is
never committed.

## Quick start

```bash
uv sync --project fetch
cargo build --release --manifest-path analyze/Cargo.toml

export GARMIN_EMAIL=you@example.com   # the password is prompted for, once
./analyze.sh
```

`./analyze.sh` fetches at most once an hour (`--force` to fetch anyway), then
prints the report and every run/walk pass of the last 7 days. `DIGIRUN_OUT` and
`DIGIRUN_INTERVAL` override the data directory and the interval.

## License

MIT, see [LICENSE](LICENSE).
