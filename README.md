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
prints the report, every run/walk pass of the last 7 days, weekly progress over the
last 8 weeks, and this week against the previous 4. `--full` swaps the last two for
every metric table over all history: progress, HR zones, fitness and best efforts.
Every run also writes `runs/analysis/report.html`, one page with charts and tables
that also works offline, apart from its satellite map; `--open` opens it when the run
ends. `--no-map`, or `DIGIRUN_MAP=0`, leaves the map and your GPS positions out.
`--name-routes` asks for a name for each route run in the last 30 days instead,
and saves them in `runs/routes.txt`.
`--serve` serves the report on http://127.0.0.1:8765/ instead (with `--open`, it
opens it), where the ✎ on a route card renames the route and saves `routes.txt`.
`DIGIRUN_OUT` and `DIGIRUN_INTERVAL` override the data directory and
the interval, and `DIGIRUN_HR_MAX` adds HR load to the weekly summary. Settings can
go in a `.env` file, which is gitignored (see [`.env.example`](.env.example)); a
variable set in the shell wins over it.

`uv run --project fetch garmin-runs --profile` prints your max HR setting, resting
HR, weight, VO2 max and fitness age from Garmin, next to the highest HR recorded on
your runs, and saves them as `runs/profile.json`.

## License

MIT, see [LICENSE](LICENSE).
