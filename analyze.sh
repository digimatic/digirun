#!/bin/bash
# Fetch at most once an hour, then analyze. The analysis is offline, so it runs
# every time even when the fetch is skipped.
set -uo pipefail

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
        uv run garmin-runs --out "$OUT"
    fi
else
    date +%s > "$STAMP"
    uv run garmin-runs --out "$OUT"
fi

uv run garmin-analyze --out "$OUT" --max-pace 15:00 --min-pace 3:00 --cadence-stats \
    --type running,trail,walking
