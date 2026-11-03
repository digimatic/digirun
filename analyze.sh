#!/bin/bash
# Fetch at most once an hour, then analyze. The analysis is offline, so it runs
# every time even when the fetch is skipped.
set -uo pipefail

# Settings from .env. A variable already set in the environment wins.
if [ -f .env ]; then
    while IFS='=' read -r key value || [ -n "$key" ]; do
        case "$key" in
            ''|\#*) continue ;;
        esac
        value="${value%$'\r'}"
        if [ -z "${!key+set}" ]; then
            export "$key=$value"
        fi
    done < .env
fi

OUT="${DIGIRUN_OUT:-./runs}"
INTERVAL="${DIGIRUN_INTERVAL:-3600}"
STAMP="$OUT/.last_fetch"
FORCE=0
FULL=0
OPEN=0
MAP="${DIGIRUN_MAP:-1}"
for arg in "$@"; do
    case "$arg" in
        --force) FORCE=1 ;;
        --full) FULL=1 ;;
        --open) OPEN=1 ;;
        --map) MAP=1 ;;
        --no-map) MAP=0 ;;
        *)
            echo "usage: $0 [--force] [--full] [--open] [--map|--no-map]" >&2
            exit 2
            ;;
    esac
done

mkdir -p "$OUT"
now=$(date +%s)

if [ -f "$STAMP" ] && [ "$FORCE" -eq 0 ]; then
    last=$(cat "$STAMP")
    age=$((now - last))
    if [ "$age" -lt "$INTERVAL" ]; then
        echo "Fetch skipped: last attempt ${age}s ago, next in $((INTERVAL - age))s (--force to fetch anyway)."
    else
        date +%s > "$STAMP"   # stamp the attempt, so failures also count against the interval
        uv run --project fetch garmin-runs --out "$OUT"
    fi
else
    date +%s > "$STAMP"
    uv run --project fetch garmin-runs --out "$OUT"
fi

# The analyzer (analyze/README.md), built once on first use.
ANALYZE=analyze/target/release/garmin-analyze
[ -x "$ANALYZE" ] || cargo build --release --manifest-path analyze/Cargo.toml

"$ANALYZE" --out "$OUT" --max-pace 15:00 --min-pace 3:00 --cadence-stats \
    --type running,trail

# Every run/walk pass of the last 7 days, split on cadence (analyze/README.md).
echo
"$ANALYZE" --out "$OUT" --max-pace 15:00 --min-pace 3:00 --show detail --days 7 \
    --type running,trail

# Set DIGIRUN_HR_MAX for HR zones and load (analyze/README.md).
echo
# The charts and tables in one page, always over all history.
REPORT="$OUT/analysis/report.html"
# The satellite map means the page also holds GPS positions (analyze/README.md);
# --no-map or DIGIRUN_MAP=0 leaves them out.
MAP_FLAG=""
if [ "$MAP" = "1" ]; then
    MAP_FLAG="--map"
fi
if [ "$FULL" -eq 1 ]; then
    # Every metric table, over all history.
    "$ANALYZE" --out "$OUT" --max-pace 15:00 --min-pace 3:00 \
        --show progress,zones,fitness,efforts,summary \
        --html "$REPORT" $MAP_FLAG --type running,trail ${DIGIRUN_HR_MAX:+--hr-max "$DIGIRUN_HR_MAX"}
else
    # The last 8 weeks, then this week against the previous 4. Only the summary
    # writes the CSVs, because they cover the range and it has none.
    "$ANALYZE" --out "$OUT" --max-pace 15:00 --min-pace 3:00 --show progress --days 56 --no-csv \
        --type running,trail
    echo
    "$ANALYZE" --out "$OUT" --max-pace 15:00 --min-pace 3:00 --show summary \
        --html "$REPORT" $MAP_FLAG --type running,trail ${DIGIRUN_HR_MAX:+--hr-max "$DIGIRUN_HR_MAX"}
fi

if [ "$OPEN" -eq 1 ]; then
    if command -v open >/dev/null; then
        open "$REPORT"
    else
        xdg-open "$REPORT"
    fi
fi
