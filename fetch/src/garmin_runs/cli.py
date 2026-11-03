"""Download runs from Garmin Connect as GPX/TCX/... files plus a summary CSV.

Credentials are read from the environment (GARMIN_EMAIL / GARMIN_PASSWORD) or
prompted interactively. Session tokens are cached in a token store so the
password is only needed on the first run.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import sys
from datetime import date, datetime, timedelta
from getpass import getpass
from pathlib import Path

from garminconnect import Garmin
from garminconnect.exceptions import (
    GarminConnectAuthenticationError,
    GarminConnectConnectionError,
    GarminConnectTooManyRequestsError,
)

DEFAULT_DAYS = 90
DEFAULT_TOKENSTORE = "~/.garminconnect"

# Map CLI format names to the library's download enum + file extension.
# Imported lazily in build_parser/format helpers to keep module import light.
FORMATS = ("gpx", "tcx", "kml", "csv", "original")
FORMAT_EXT = {
    "gpx": ".gpx",
    "tcx": ".tcx",
    "kml": ".kml",
    "csv": ".csv",
    "original": ".zip",
}

# The default "runs" bucket. Garmin tags trail running with its own type key,
# so include both to capture every kind of run.
RUNS_TYPES = {"running", "trail"}

SUMMARY_FIELDS = [
    "date",
    "start_local",
    "activity_id",
    "name",
    "type",
    "distance_km",
    "moving_time",
    "avg_pace_s_per_km",
    "elevation_gain_m",
    "avg_hr",
    "max_hr",
    "calories",
]


def _type_key(activity: dict) -> str:
    """Return the activity type key, e.g. 'running' or 'trail'."""
    return str(activity.get("activityType", {}).get("typeKey", "unknown"))


def _safe_name(name: str) -> str:
    """Sanitize an activity name for use as a filename component."""
    name = (name or "activity").strip()
    name = re.sub(r"[^A-Za-z0-9._-]+", "-", name)
    name = re.sub(r"-{2,}", "-", name).strip("-.")
    return name or "activity"


def _fmt_hhmmss(seconds: float | None) -> str:
    if not seconds or seconds <= 0:
        return ""
    seconds = int(round(seconds))
    h, rem = divmod(seconds, 3600)
    m, s = divmod(rem, 60)
    if h:
        return f"{h}:{m:02d}:{s:02d}"
    return f"{m}:{s:02d}"


def resolve_date_range(args: argparse.Namespace) -> tuple[str, str]:
    """Return (start, end) as ISO YYYY-MM-DD strings."""
    today = date.today()
    if args.days:
        end = today
        start = today - timedelta(days=args.days - 1)
    elif args.start or args.end:
        start = _parse_date(args.start) if args.start else today - timedelta(days=DEFAULT_DAYS - 1)
        end = _parse_date(args.end) if args.end else today
        if start > end:
            raise SystemExit(f"error: --start {start} is after --end {end}")
    else:
        end = today
        start = today - timedelta(days=DEFAULT_DAYS - 1)
    return start.isoformat(), end.isoformat()


def _parse_date(value: str) -> date:
    try:
        return datetime.strptime(value, "%Y-%m-%d").date()
    except ValueError as e:
        raise SystemExit(f"error: invalid date {value!r}, expected YYYY-MM-DD") from e


def _type_filter(args: argparse.Namespace) -> tuple[str | None, set[str] | None]:
    """Return (server_type, client_filter) for the requested --type.

    - 'runs' (default): no server filter, keep {running, trail} client-side.
    - 'all': no filtering at all.
    - a concrete type: pass it to the server and keep exactly that type.
    """
    t = args.type
    if t == "runs":
        return None, RUNS_TYPES
    if t == "all":
        return None, None
    return t, {t}


def get_credentials(args: argparse.Namespace) -> tuple[str, str, bool]:
    email = os.getenv("GARMIN_EMAIL") or input("Garmin email: ").strip()
    password = os.getenv("GARMIN_PASSWORD")
    if not password:
        password = getpass("Garmin password: ")
    is_cn = args.cn or os.getenv("GARMIN_IS_CN", "").lower() in {"1", "true", "yes"}
    if not email or not password:
        raise SystemExit("error: email and password are required")
    return email, password, is_cn


def make_client(args: argparse.Namespace, email: str, password: str, is_cn: bool) -> Garmin:
    prompt_mfa = None if os.getenv("GARMIN_MFA") else (lambda: input("MFA code: ").strip())
    return Garmin(email, password, is_cn=is_cn, prompt_mfa=prompt_mfa)


def summarize(activity: dict) -> dict:
    """Build a summary row from an activity dict."""
    distance_m = activity.get("distance") or 0.0
    moving = activity.get("movingDuration") or activity.get("duration") or 0.0
    start_local = activity.get("startTimeLocal", "")
    pace = None
    if distance_m and moving:
        pace = round(moving / (distance_m / 1000.0), 1)
    return {
        "date": start_local[:10],
        "start_local": start_local,
        "activity_id": activity.get("activityId"),
        "name": activity.get("activityName", ""),
        "type": _type_key(activity),
        "distance_km": round(distance_m / 1000.0, 2) if distance_m else "",
        "moving_time": _fmt_hhmmss(moving) if moving else "",
        "avg_pace_s_per_km": pace if pace is not None else "",
        "elevation_gain_m": activity.get("elevationGain") or "",
        "avg_hr": activity.get("averageHR") or "",
        "max_hr": activity.get("maxHR") or "",
        "calories": activity.get("calories") or "",
    }


def load_summary(path: Path) -> dict[str, dict]:
    """Load an existing summary.csv keyed by activity_id (if present)."""
    rows: dict[str, dict] = {}
    if not path.exists():
        return rows
    try:
        with path.open(newline="", encoding="utf-8") as f:
            for row in csv.DictReader(f):
                aid = row.get("activity_id")
                if aid:
                    rows[aid] = row
    except Exception:
        # Corrupt/unreadable summary: start fresh rather than crash.
        return {}
    return rows


def write_summary(path: Path, rows: dict[str, dict]) -> None:
    ordered = sorted(rows.values(), key=lambda r: (r.get("date", ""), r.get("start_local", "")))
    with path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=SUMMARY_FIELDS, extrasaction="ignore")
        writer.writeheader()
        for row in ordered:
            writer.writerow(row)


def _first_hr_zones(zones: list[dict] | None) -> dict:
    """The zones for running, else the default profile, else whichever comes first."""
    rows = zones or []
    for sport in ("RUNNING", "DEFAULT"):
        for row in rows:
            if row.get("sport") == sport:
                return row
    return rows[0] if rows else {}


def _weight_kg(grams: float | None) -> float | None:
    """Garmin keeps the profile weight in grams."""
    return round(grams / 1000.0, 1) if grams else None


def extract_profile(zones: list[dict] | None, user: dict | None, metrics: list[dict] | None,
                    resting: dict | None) -> dict:
    """Pick the training fields from Garmin's responses.

    This is an explicit list on purpose: the responses also hold the birth date,
    home location and other details that have no use here and should not be saved.
    """
    hr = _first_hr_zones(zones)
    data = (user or {}).get("userData") or {}
    generic = ((metrics or [{}])[0] or {}).get("generic") or {}
    resting_rows = (((resting or {}).get("allMetrics") or {}).get("metricsMap") or {}).get(
        "WELLNESS_RESTING_HEART_RATE") or [{}]
    return {
        "max_hr_setting": hr.get("maxHeartRateUsed"),
        "resting_hr_setting": hr.get("restingHeartRateUsed"),
        "resting_hr_today": resting_rows[0].get("value"),
        "lactate_threshold_hr": hr.get("lactateThresholdHeartRateUsed") or data.get("lactateThresholdHeartRate"),
        "zone_method": hr.get("trainingMethod"),
        "zone_floors": [hr[f"zone{i}Floor"] for i in range(1, 6) if f"zone{i}Floor" in hr],
        "weight_kg": _weight_kg(data.get("weight")),
        "height_cm": data.get("height"),
        "vo2max_running": generic.get("vo2MaxPreciseValue") or data.get("vo2MaxRunning"),
        "fitness_age": generic.get("fitnessAge"),
    }


def recorded_max_hr(summary_path: Path, top: int = 3) -> list[tuple[int, str]]:
    """The highest max HR recorded on runs in summary.csv, as (bpm, date)."""
    rows = []
    for row in load_summary(summary_path).values():
        if row.get("type") in RUNS_TYPES and row.get("max_hr"):
            rows.append((int(float(row["max_hr"])), row.get("date", "")))
    return sorted(rows, reverse=True)[:top]


def _ask(label: str, call):
    """One Garmin call; a failure blanks its fields rather than losing the others."""
    try:
        return call()
    except (GarminConnectConnectionError, GarminConnectTooManyRequestsError) as e:
        print(f"  ! {label}: {e}", file=sys.stderr)
        return None


def run_profile(client: Garmin, out_dir: Path) -> int:
    """Print the training fields of the Garmin profile and save them as profile.json."""
    today = date.today().isoformat()
    profile = extract_profile(
        _ask("heart rate zones", client.get_heart_rate_zones),
        _ask("user profile", client.get_user_profile),
        _ask("max metrics", lambda: client.get_max_metrics(today)),
        _ask("resting heart rate", lambda: client.get_rhr_day(today)),
    )
    labels = {
        "max_hr_setting": "Max HR (Garmin setting)",
        "resting_hr_setting": "Resting HR (Garmin setting)",
        "resting_hr_today": "Resting HR (today)",
        "lactate_threshold_hr": "Lactate threshold HR",
        "zone_method": "Zone method",
        "zone_floors": "Zone floors",
        "weight_kg": "Weight (kg)",
        "height_cm": "Height (cm)",
        "vo2max_running": "VO2 max (running)",
        "fitness_age": "Fitness age",
    }
    for key, label in labels.items():
        value = profile.get(key)
        print(f"{label:<28} {value if value not in (None, []) else '-'}")

    out_dir.mkdir(parents=True, exist_ok=True)
    recorded = recorded_max_hr(out_dir / "summary.csv")
    if recorded:
        shown = ", ".join(f"{bpm} ({day})" for bpm, day in recorded)
        print(f"{'Highest recorded on runs':<28} {shown}")

    path = out_dir / "profile.json"
    path.write_text(json.dumps({"fetched": today, **profile}, indent=2) + "\n", encoding="utf-8")
    print(f"\nSaved {path}. Zones use DIGIRUN_HR_MAX from .env, which is never set for you.")
    return 0


def output_path(out_dir: Path, activity: dict, fmt: str) -> Path:
    start_local = activity.get("startTimeLocal", "")
    stamp = ""
    if start_local:
        try:
            dt = datetime.strptime(start_local[:19], "%Y-%m-%d %H:%M:%S")
            stamp = dt.strftime("%Y-%m-%d_%H%M")
        except ValueError:
            stamp = start_local.replace(":", "").replace(" ", "_")[:15]
    base = f"{stamp}_{_safe_name(activity.get('activityName', ''))}_{activity.get('activityId', 'activity')}"
    return out_dir / f"{base}{FORMAT_EXT[fmt]}"


def fmt_enum(fmt: str):
    from garminconnect import Garmin as _G

    return _G.ActivityDownloadFormat[fmt.upper()]


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    email, password, is_cn = get_credentials(args)
    tokenstore = args.tokenstore or os.getenv("GARMINTOKENS") or DEFAULT_TOKENSTORE

    try:
        client = make_client(args, email, password, is_cn)
        needs_mfa, _ = client.login(tokenstore)
        if needs_mfa:
            print("MFA required; please re-run and enter the code when prompted.", file=sys.stderr)
            return 1
    except (GarminConnectAuthenticationError, GarminConnectTooManyRequestsError) as e:
        print(f"login failed: {e}", file=sys.stderr)
        return 1
    except GarminConnectConnectionError as e:
        print(f"connection error: {e}", file=sys.stderr)
        return 1

    if args.profile:
        return run_profile(client, Path(args.out).expanduser())

    start, end = resolve_date_range(args)
    server_type, client_filter = _type_filter(args)
    print(f"Fetching activities {start} → {end} (type={args.type}) ...")

    try:
        activities = client.get_activities_by_date(start, end, server_type)
    except (GarminConnectConnectionError, GarminConnectTooManyRequestsError) as e:
        print(f"failed to fetch activities: {e}", file=sys.stderr)
        return 1

    if client_filter is not None:
        activities = [a for a in activities if _type_key(a) in client_filter]

    if not activities:
        print("No activities found in that range.")
        return 0

    # Sort oldest → newest for predictable filenames/summary.
    activities.sort(key=lambda a: a.get("startTimeLocal", ""))

    out_dir = Path(args.out).expanduser()
    out_dir.mkdir(parents=True, exist_ok=True)

    summary = load_summary(out_dir / "summary.csv")
    downloaded = skipped = failed = 0
    formats = [f for f in (x.strip() for x in args.format.split(",")) if f]

    for a in activities:
        aid = a.get("activityId")
        name = a.get("activityName", "")
        for fmt in formats:
            dest = output_path(out_dir, a, fmt)
            if dest.exists():
                skipped += 1
                continue
            try:
                data = client.download_activity(str(aid), dl_fmt=fmt_enum(fmt))
                dest.write_bytes(data)
                downloaded += 1
            except Exception as e:  # noqa: BLE001 — keep the batch going
                failed += 1
                print(f"  ! {name} [{fmt}]: {e}", file=sys.stderr)

        summary[str(aid)] = summarize(a)

    write_summary(out_dir / "summary.csv", summary)

    print(f"Done. {len(activities)} activities, {downloaded} files written, "
          f"{skipped} already present, {failed} failed. Summary: {out_dir / 'summary.csv'}")
    return 0 if failed == 0 else 2


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="garmin-runs",
        description="Download runs from Garmin Connect as GPX/TCX/... files plus a summary CSV.",
    )
    rng = p.add_argument_group("date range")
    rng.add_argument("--days", type=int, metavar="N",
                     help=f"download the last N days (default: {DEFAULT_DAYS} when no range given)")
    rng.add_argument("--start", metavar="YYYY-MM-DD", help="start date (inclusive)")
    rng.add_argument("--end", metavar="YYYY-MM-DD", help="end date (inclusive, default: today)")

    sel = p.add_argument_group("selection")
    sel.add_argument("--type", default="runs",
                     help="activity type: 'runs' (running+trail, default), 'all', "
                          "or a single type such as running, trail, cycling, swimming, ...")
    sel.add_argument("--format", default="gpx,tcx",
                     help="comma-separated output formats: " + ",".join(FORMATS) + " (default: gpx,tcx)")

    out = p.add_argument_group("output")
    out.add_argument("--out", default="./runs", help="output directory (default: ./runs)")
    out.add_argument("--profile", action="store_true",
                     help="print max HR, resting HR, weight, VO2 max and so on, and save them "
                          "as profile.json in --out, instead of downloading activities")

    auth = p.add_argument_group("authentication")
    auth.add_argument("--tokenstore", default=None,
                      help="token cache path (default: ~/.garminconnect or $GARMINTOKENS)")
    auth.add_argument("--cn", action="store_true",
                      help="use the Garmin China (garmin.cn) endpoints")
    return p
