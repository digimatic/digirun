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
[ "${1:-}" = "--force" ] && FORCE=1

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

# This week against the previous 4 weeks. Set DIGIRUN_HR_MAX for the HR load.
echo
"$ANALYZE" --out "$OUT" --max-pace 15:00 --min-pace 3:00 --show summary \
    --type running,trail ${DIGIRUN_HR_MAX:+--hr-max "$DIGIRUN_HR_MAX"}
