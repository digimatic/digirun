"""Analyze downloaded GPX/TCX files: split each activity into walk and run segments.

Reads the files already written by garmin-runs (no network access), splits each
activity into time windows, classifies every window as walk or run, and reports
mean pace plus standard deviation per class — per activity, per week, and pooled
overall.

Classification is threshold-based: pace is derived from position deltas (GPX has
no speed field), cadence comes from the trackpoint extensions when present.
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
from dataclasses import dataclass
from datetime import date, datetime
from pathlib import Path
from statistics import mean, stdev
from xml.etree import ElementTree as ET

DEFAULT_OUT = "./runs"
DEFAULT_REPORT_DIR = "./runs/analysis"

DEFAULT_WINDOW = 30            # seconds of track per segment
DEFAULT_MIN_SEGMENT = 20       # ignore windows shorter than this
DEFAULT_MIN_DISTANCE_M = 3.0   # ignore windows shorter than this
DEFAULT_GAP_S = 60             # a longer pause ends the current window
DEFAULT_CADENCE_THRESHOLD = 65  # strides/min; at or above this is a run
DEFAULT_PACE_THRESHOLD = 540    # s/km; faster than this is a run
AMBIGUOUS_CADENCE = (61, 71)    # hybrid resolves this band with the pace threshold
DEFAULT_TYPES = "running,trail,walking"  # activity types analyzed by default

METHODS = ("hybrid", "cadence", "pace", "lap")

ANALYSIS_FIELDS = [
    "date",
    "activity_id",
    "name",
    "type",
    "class",
    "segments",
    "time_s",
    "distance_km",
    "mean_pace_s_per_km",
    "stdev_pace_s_per_km",
    "min_pace_s_per_km",
    "max_pace_s_per_km",
    "share_pct",
]
# Added to analysis.csv only with --cadence-stats.
CADENCE_ANALYSIS_FIELDS = ["mean_cadence", "stdev_cadence"]
SEGMENT_FIELDS = [
    "activity_id",
    "date",
    "time",
    "class",
    "pace_s_per_km",
    "cadence",
    "distance_m",
    "window_s",
]


@dataclass
class Point:
    time: datetime
    lat: float
    lon: float
    distance_m: float
    cadence: float | None


@dataclass
class Lap:
    start_index: int
    end_index: int
    duration_s: float
    distance_m: float


@dataclass
class Activity:
    activity_id: str
    name: str
    date: date
    points: list[Point]
    laps: list[Lap]
    type: str = ""


@dataclass
class Segment:
    activity: Activity
    time: datetime
    duration_s: float
    distance_m: float
    pace_s_per_km: float
    cadence: float | None
    klass: str


def _local(tag: str) -> str:
    """Element tag without its XML namespace."""
    return tag.split("}")[-1]


def _find(elem, name: str):
    """First child element with the given local name (namespace-agnostic)."""
    if elem is None:
        return None
    for child in elem:
        if _local(child.tag) == name:
            return child
    return None


def _text(elem, name: str) -> str:
    child = _find(elem, name)
    return (child.text or "").strip() if child is not None and child.text else ""


def _parse_time(value: str) -> datetime | None:
    if not value:
        return None
    try:
        return datetime.fromisoformat(value.strip().replace("Z", "+00:00"))
    except ValueError:
        return None


def _num(value: str) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def haversine(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    """Great-circle distance in metres."""
    r = 6371000.0
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dp = p2 - p1
    dl = math.radians(lon2 - lon1)
    h = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * r * math.asin(math.sqrt(h))


def _activity_id(path: Path) -> str:
    """Activity id from the download filename (trailing _<id>), else the stem."""
    parts = path.stem.rsplit("_", 1)
    return parts[1] if len(parts) == 2 and parts[1].isdigit() else path.stem


def activity_types(out_dir: Path) -> dict[str, str]:
    """activity_id -> type key, from the summary.csv written by garmin-runs.

    The GPX/TCX files themselves do not record the activity type, so summary.csv
    is the only source. Returns {} when it is missing.
    """
    path = out_dir / "summary.csv"
    if not path.is_file():
        return {}
    with path.open(encoding="utf-8") as f:
        return {row["activity_id"]: row.get("type", "") for row in csv.DictReader(f)}


def read_gpx(path: Path) -> tuple[list[Point], str]:
    """Trackpoints plus the track name. Distance is derived from positions."""
    root = ET.parse(path).getroot()
    trk = _find(root, "trk")
    name = _text(trk, "name")
    points: list[Point] = []
    if trk is None:
        return points, name
    cumulative = 0.0
    prev: Point | None = None
    for seg in trk:
        if _local(seg.tag) != "trkseg":
            continue
        for pt in seg:
            if _local(pt.tag) != "trkpt":
                continue
            lat = float(pt.get("lat") or 0.0)
            lon = float(pt.get("lon") or 0.0)
            time = _parse_time(_text(pt, "time"))
            if time is None:
                continue
            if prev is not None:
                cumulative += haversine(prev.lat, prev.lon, lat, lon)
            ext = _find(pt, "extensions")
            tpx = _find(ext, "TrackPointExtension") if ext is not None else None
            cadence = _num(_text(tpx, "cad")) if tpx is not None else None
            point = Point(time, lat, lon, cumulative, cadence)
            points.append(point)
            prev = point
    return points, name


def read_tcx(path: Path) -> tuple[list[Point], list[Lap]]:
    """Trackpoints and laps. DistanceMeters is cumulative within each lap."""
    root = ET.parse(path).getroot()
    activity_el = _find(_find(root, "Activities"), "Activity")
    points: list[Point] = []
    laps: list[Lap] = []
    if activity_el is None:
        return points, laps
    for lap in activity_el:
        if _local(lap.tag) != "Lap":
            continue
        start_index = len(points)
        duration = _num(_text(lap, "TotalTimeSeconds")) or 0.0
        distance = _num(_text(lap, "DistanceMeters")) or 0.0
        track = _find(lap, "Track")
        for tp in (track or []):
            if _local(tp.tag) != "Trackpoint":
                continue
            time = _parse_time(_text(tp, "Time"))
            if time is None:
                continue
            # Position is optional: some exports have no GPS, so DistanceMeters
            # (cumulative within the lap) is the only distance available.
            pos = _find(tp, "Position")
            lat = float(_text(pos, "LatitudeDegrees") or 0.0) if pos is not None else 0.0
            lon = float(_text(pos, "LongitudeDegrees") or 0.0) if pos is not None else 0.0
            ext = _find(tp, "Extensions")
            tpx = _find(ext, "TPX") if ext is not None else None
            cadence = _num(_text(tpx, "RunCadence")) if tpx is not None else None
            points.append(
                Point(
                    time,
                    lat,
                    lon,
                    _num(_text(tp, "DistanceMeters")) or 0.0,
                    cadence,
                )
            )
        if len(points) > start_index:
            laps.append(Lap(start_index, len(points) - 1, duration, distance))
    return points, laps


def classify(pace: float, cadence: float | None, args: argparse.Namespace) -> str:
    """Return 'run' or 'walk' for one segment."""
    if args.method == "pace":
        return "run" if pace <= args.pace_threshold else "walk"
    if args.method == "cadence":
        if cadence is None:
            return "run" if pace <= args.pace_threshold else "walk"
        return "run" if cadence >= args.cadence_threshold else "walk"
    if args.method == "hybrid":
        if cadence is not None:
            lo, hi = AMBIGUOUS_CADENCE
            if cadence < lo:
                return "walk"
            if cadence > hi:
                return "run"
        return "run" if pace <= args.pace_threshold else "walk"
    return "run" if pace <= args.pace_threshold else "walk"


def build_segments(activity: Activity, args: argparse.Namespace) -> list[Segment]:
    """Split an activity into non-overlapping windows of `--window` seconds."""
    segments: list[Segment] = []
    bucket: list[tuple[float, float, float | None]] = []
    prev: Point | None = None

    for point in activity.points:
        if prev is not None:
            dt = (point.time - prev.time).total_seconds()
            dm = point.distance_m - prev.distance_m
            if dt > args.gap:
                bucket.clear()  # stopped time: do not count it as walking
            elif dm < 0:
                bucket.clear()  # TCX DistanceMeters resets at a lap boundary
            elif dt > 0 and dm > 0:
                bucket.append((dt, dm, point.cadence))
        prev = point

        total_time = sum(x[0] for x in bucket)
        if total_time < args.window:
            continue
        total_distance = sum(x[1] for x in bucket)
        if total_time >= args.min_segment and total_distance >= args.min_distance:
            cadences = [x[2] for x in bucket if x[2] is not None]
            pace = total_time / (total_distance / 1000.0)
            segments.append(
                Segment(
                    activity=activity,
                    time=point.time,
                    duration_s=total_time,
                    distance_m=total_distance,
                    pace_s_per_km=pace,
                    cadence=mean(cadences) if cadences else None,
                    klass=classify(pace, mean(cadences) if cadences else None, args),
                )
            )
            bucket.clear()
    return segments


def lap_segments(activity: Activity, args: argparse.Namespace) -> list[Segment]:
    """Treat each TCX lap as one segment, labelled by its own pace."""
    segments: list[Segment] = []
    for lap in activity.laps:
        if lap.duration_s <= 0 or lap.distance_m <= 0:
            continue
        pace = lap.duration_s / (lap.distance_m / 1000.0)
        cadences = [
            activity.points[i].cadence
            for i in range(lap.start_index, lap.end_index + 1)
            if activity.points[i].cadence is not None
        ]
        segments.append(
            Segment(
                activity=activity,
                time=activity.points[lap.start_index].time,
                duration_s=lap.duration_s,
                distance_m=lap.distance_m,
                pace_s_per_km=pace,
                cadence=mean(cadences) if cadences else None,
                klass=classify(pace, mean(cadences) if cadences else None, args),
            )
        )
    return segments


def fmt_pace(seconds: float) -> str:
    """Pace as M:SS per kilometre."""
    m, s = divmod(int(round(seconds)), 60)
    return f"{m}:{s:02d}"


def fmt_duration(seconds: float) -> str:
    """Total time as H:MM:SS, or M:SS under an hour."""
    seconds = int(round(seconds))
    h, rem = divmod(seconds, 3600)
    m, s = divmod(rem, 60)
    if h:
        return f"{h}:{m:02d}:{s:02d}"
    return f"{m}:{s:02d}"


def stats(groups: list[Segment]) -> dict:
    """Mean/stdev of pace, plus cadence where the segments have it."""
    paces = [s.pace_s_per_km for s in groups]
    cadences = [s.cadence for s in groups if s.cadence is not None]
    return {
        "segments": len(paces),
        "time_s": round(sum(s.duration_s for s in groups), 1),
        "distance_km": round(sum(s.distance_m for s in groups) / 1000.0, 2),
        "mean": round(mean(paces), 1) if paces else 0.0,
        "stdev": round(stdev(paces), 1) if len(paces) > 1 else 0.0,
        "min": round(min(paces), 1) if paces else 0.0,
        "max": round(max(paces), 1) if paces else 0.0,
        "cad_mean": round(mean(cadences), 1) if cadences else None,
        "cad_stdev": round(stdev(cadences), 1) if len(cadences) > 1 else None,
    }


def _name_from_filename(path: Path) -> str:
    """Activity name from the download filename, without the timestamp prefix."""
    stem = path.stem
    parts = stem.split("_")
    if len(parts) >= 3:
        stem = "_".join(parts[2:])
    return stem.rsplit("_", 1)[0] or path.stem


def _date_from_filename(path: Path) -> date | None:
    try:
        return datetime.strptime(path.stem[:10], "%Y-%m-%d").date()
    except ValueError:
        return None


def load_activities(out_dir: Path) -> tuple[list[Activity], list[str]]:
    """Group the downloaded files by activity id and read them into activities."""
    groups: dict[str, dict[str, Path]] = {}
    for path in sorted(out_dir.glob("*")):
        if path.suffix.lower() not in (".gpx", ".tcx"):
            continue
        groups.setdefault(_activity_id(path), {})[path.suffix.lower()] = path

    activities: list[Activity] = []
    skipped: list[str] = []
    for aid, files in sorted(groups.items()):
        gpx_points, gpx_name = read_gpx(files[".gpx"]) if ".gpx" in files else ([], "")
        tcx_points, tcx_laps = read_tcx(files[".tcx"]) if ".tcx" in files else ([], [])
        # TCX is preferred: it carries Garmin's own distances, laps and speed.
        points = tcx_points or gpx_points
        if not points:
            skipped.append(aid)
            continue
        name = gpx_name or _name_from_filename(files.get(".gpx") or files.get(".tcx"))
        when = _date_from_filename(files.get(".gpx") or files.get(".tcx")) or points[0].time.date()
        activities.append(Activity(aid, name, when, points, tcx_laps))

    activities.sort(key=lambda a: (a.date, a.activity_id))
    return activities, skipped


def in_range(activity: Activity, args: argparse.Namespace) -> bool:
    if args.start and activity.date < args.start:
        return False
    if args.end and activity.date > args.end:
        return False
    return True


def method_note(args: argparse.Namespace) -> str:
    pace = fmt_pace(args.pace_threshold)
    if args.method == "cadence":
        return f"cadence >= {args.cadence_threshold} strides/min (pace fallback: {pace}/km)"
    if args.method == "pace":
        return f"pace <= {pace}/km"
    if args.method == "lap":
        return f"TCX laps, each labelled by pace <= {pace}/km"
    lo, hi = AMBIGUOUS_CADENCE
    return (
        f"cadence < {lo} walk / > {hi} run, band {lo}-{hi} resolved by pace <= {pace}/km"
    )


def filter_activities(
    activities: list[Activity], wanted: set[str] | None
) -> tuple[list[Activity], int, int]:
    """Keep activities whose type is in `wanted`; returns (kept, skipped, unknown).

    Activities whose type is unknown (not in summary.csv) are skipped, since the
    point of the filter is to keep other sports out of the stats.
    """
    if not wanted:
        return activities, 0, 0
    kept: list[Activity] = []
    skipped = unknown = 0
    for activity in activities:
        if not activity.type:
            unknown += 1
        elif activity.type in wanted:
            kept.append(activity)
        else:
            skipped += 1
    return kept, skipped, unknown


def filter_segments(segments: list[Segment], args: argparse.Namespace) -> tuple[list[Segment], int, int]:
    """Drop segments outside the pace window; returns (kept, dropped_slow, dropped_fast)."""
    if not args.max_pace and not args.min_pace:
        return segments, 0, 0
    kept: list[Segment] = []
    slow = fast = 0
    for s in segments:
        if args.max_pace and s.pace_s_per_km > args.max_pace:
            slow += 1
        elif args.min_pace and s.pace_s_per_km < args.min_pace:
            fast += 1
        else:
            kept.append(s)
    return kept, slow, fast


def _cell(value) -> str:
    """Right-aligned cell, or '-' when the value is missing."""
    return f"{value:>6}" if value is not None else f"{'-':>6}"


def print_report(
    activities: list[Activity],
    segments: list[Segment],
    args: argparse.Namespace,
    skipped: list[str],
    notes: list[str] = (),
) -> None:
    show_cad = args.cadence_stats

    def cells(st: dict) -> list[str]:
        out = [f"{st['segments']:>6}", f"{fmt_pace(st['mean']):>7}", f"{fmt_pace(st['stdev']):>6}"]
        if show_cad:
            out += [_cell(st["cad_mean"]), _cell(st["cad_stdev"])]
        return out

    def table_header(leading: list[str]) -> str:
        parts = list(leading)
        parts += [f"{'run n':>6}", f"{'run':>7}", f"{'sd':>6}"]
        if show_cad:
            parts += [f"{'cad':>6}", f"{'sd':>6}"]
        parts += [f"{'walk n':>6}", f"{'walk':>7}", f"{'sd':>6}"]
        if show_cad:
            parts += [f"{'cad':>6}", f"{'sd':>6}"]
        return " ".join(parts)

    print(f"Method: {args.method} — {method_note(args)}")
    print(
        f"Window: {args.window}s, min segment {args.min_segment}s / {args.min_distance}m, "
        f"gap {args.gap}s"
    )
    for note in notes:
        print(note)
    print()

    by_activity: dict[str, list[Segment]] = {}
    for s in segments:
        by_activity.setdefault(s.activity.activity_id, []).append(s)

    header = table_header([f"{'date':<11}", f"{'type':<8}", f"{'activity':<34}"])
    print(header)
    print("-" * len(header))
    for activity in activities:
        rows = by_activity.get(activity.activity_id, [])
        if not rows:
            continue
        run = stats([s for s in rows if s.klass == "run"])
        walk = stats([s for s in rows if s.klass == "walk"])
        print(
            " ".join(
                [
                    f"{activity.date.isoformat():<11}",
                    f"{activity.type or '-':<8}",
                    f"{activity.name[:34]:<34}",
                ]
                + cells(run)
                + cells(walk)
            )
        )

    print()
    print("Weekly")
    header = table_header([f"{'week':<10}"])
    print(header)
    print("-" * len(header))
    weeks: dict[str, list[Segment]] = {}
    for s in segments:
        iso = s.activity.date.isocalendar()
        weeks.setdefault(f"{iso[0]}-W{iso[1]:02d}", []).append(s)
    for week in sorted(weeks):
        rows = weeks[week]
        run = stats([s for s in rows if s.klass == "run"])
        walk = stats([s for s in rows if s.klass == "walk"])
        print(" ".join([f"{week:<10}"] + cells(run) + cells(walk)))

    print()
    print("Overall")
    header = (
        f"{'class':<6} {'n':>6} {'time':>7} {'dist km':>8} {'mean':>7} {'sd':>6} "
        f"{'min':>7} {'max':>7} {'share':>6}"
    )
    if show_cad:
        header += f" {'cad':>6} {'sd':>6}"
    print(header)
    print("-" * len(header))
    total_time = sum(s.duration_s for s in segments)
    for klass in ("run", "walk"):
        rows = [s for s in segments if s.klass == klass]
        if not rows:
            continue
        st = stats(rows)
        share = st["time_s"] / total_time * 100 if total_time else 0.0
        line = (
            f"{klass:<6} {st['segments']:>6} {fmt_duration(st['time_s']):>7} {st['distance_km']:>8.2f} "
            f"{fmt_pace(st['mean']):>7} {fmt_pace(st['stdev']):>6} {fmt_pace(st['min']):>7} "
            f"{fmt_pace(st['max']):>7} {share:>5.1f}%"
        )
        if show_cad:
            line += f" {_cell(st['cad_mean'])} {_cell(st['cad_stdev'])}"
        print(line)

    if skipped:
        print()
        print(f"Skipped (no trackpoints): {', '.join(skipped)}")


def write_csvs(report_dir: Path, activities: list[Activity], segments: list[Segment], args: argparse.Namespace) -> None:
    report_dir.mkdir(parents=True, exist_ok=True)

    by_activity: dict[str, list[Segment]] = {}
    for s in segments:
        by_activity.setdefault(s.activity.activity_id, []).append(s)

    fields = ANALYSIS_FIELDS + (CADENCE_ANALYSIS_FIELDS if args.cadence_stats else [])
    analysis_path = report_dir / "analysis.csv"
    with analysis_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fields)
        writer.writeheader()
        for activity in activities:
            rows = by_activity.get(activity.activity_id, [])
            if not rows:
                continue
            total_time = sum(s.duration_s for s in rows)
            for klass in ("run", "walk"):
                group = [s for s in rows if s.klass == klass]
                if not group:
                    continue
                st = stats(group)
                row = {
                    "date": activity.date.isoformat(),
                    "activity_id": activity.activity_id,
                    "name": activity.name,
                    "type": activity.type,
                    "class": klass,
                    "segments": st["segments"],
                    "time_s": st["time_s"],
                    "distance_km": st["distance_km"],
                    "mean_pace_s_per_km": st["mean"],
                    "stdev_pace_s_per_km": st["stdev"],
                    "min_pace_s_per_km": st["min"],
                    "max_pace_s_per_km": st["max"],
                    "share_pct": round(st["time_s"] / total_time * 100, 1) if total_time else "",
                }
                if args.cadence_stats:
                    row["mean_cadence"] = st["cad_mean"] if st["cad_mean"] is not None else ""
                    row["stdev_cadence"] = st["cad_stdev"] if st["cad_stdev"] is not None else ""
                writer.writerow(row)

    segment_path = report_dir / "segments.csv"
    with segment_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=SEGMENT_FIELDS)
        writer.writeheader()
        for s in sorted(segments, key=lambda s: (s.activity.date, s.time)):
            writer.writerow(
                {
                    "activity_id": s.activity.activity_id,
                    "date": s.activity.date.isoformat(),
                    "time": s.time.isoformat(),
                    "class": s.klass,
                    "pace_s_per_km": round(s.pace_s_per_km, 1),
                    "cadence": round(s.cadence, 1) if s.cadence is not None else "",
                    "distance_m": round(s.distance_m, 1),
                    "window_s": round(s.duration_s, 1),
                }
            )

    print()
    print(f"Wrote {analysis_path} and {segment_path}")


def _parse_pace(value: str, name: str = "pace") -> int:
    """Accept '9:00', '9:30' or plain seconds."""
    if ":" in value:
        parts = value.split(":")
        try:
            return int(parts[0]) * 60 + int(parts[1])
        except ValueError as e:
            raise SystemExit(f"error: invalid {name} {value!r}, expected M:SS") from e
    try:
        return int(float(value))
    except ValueError as e:
        raise SystemExit(f"error: invalid {name} {value!r}, expected M:SS") from e


def _parse_date(value: str, name: str) -> date:
    """Accept YYYY-MM-DD."""
    try:
        return date.fromisoformat(value)
    except ValueError as e:
        raise SystemExit(f"error: invalid {name} {value!r}, expected YYYY-MM-DD") from e


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="garmin-analyze",
        description="Split downloaded runs into walk and run segments and report pace mean + stdev per class.",
    )
    src = p.add_argument_group("input")
    src.add_argument("--out", default=DEFAULT_OUT, help=f"directory of downloaded files (default: {DEFAULT_OUT})")
    src.add_argument("--start", metavar="YYYY-MM-DD", help="only activities from this date")
    src.add_argument("--end", metavar="YYYY-MM-DD", help="only activities up to this date")
    src.add_argument("--type", default=DEFAULT_TYPES,
                     help=f"activity types to analyze: 'all' or comma-separated keys "
                          f"(default: {DEFAULT_TYPES}); types come from summary.csv")

    cls = p.add_argument_group("classification")
    cls.add_argument("--method", choices=METHODS, default="hybrid",
                     help="how to tell walk from run (default: hybrid)")
    cls.add_argument("--cadence-threshold", type=int, default=DEFAULT_CADENCE_THRESHOLD,
                     help=f"strides/min at or above which a segment is a run (default: {DEFAULT_CADENCE_THRESHOLD})")
    cls.add_argument("--pace-threshold", default="9:00",
                     help=f"pace cutoff as M:SS or seconds (default: {DEFAULT_PACE_THRESHOLD}s = 9:00/km)")

    seg = p.add_argument_group("segmentation")
    seg.add_argument("--window", type=int, default=DEFAULT_WINDOW,
                     help=f"seconds of track per segment (default: {DEFAULT_WINDOW})")
    seg.add_argument("--min-segment", type=int, default=DEFAULT_MIN_SEGMENT,
                     help=f"ignore segments shorter than this many seconds (default: {DEFAULT_MIN_SEGMENT})")
    seg.add_argument("--min-distance", type=float, default=DEFAULT_MIN_DISTANCE_M,
                     help=f"ignore segments shorter than this many metres (default: {DEFAULT_MIN_DISTANCE_M})")
    seg.add_argument("--gap", type=int, default=DEFAULT_GAP_S,
                     help=f"a pause longer than this ends the segment (default: {DEFAULT_GAP_S}s)")
    seg.add_argument("--max-pace", default=None, metavar="M:SS",
                     help="exclude segments slower than this from the stats, e.g. 15:00 "
                          "(useful for filtering out standing-about segments)")
    seg.add_argument("--min-pace", default=None, metavar="M:SS",
                     help="exclude segments faster than this from the stats, e.g. 3:00 "
                          "(drops GPS-glitch segments no human could cover)")

    out = p.add_argument_group("output")
    out.add_argument("--report-dir", default=DEFAULT_REPORT_DIR,
                     help=f"where analysis.csv and segments.csv are written (default: {DEFAULT_REPORT_DIR})")
    out.add_argument("--cadence-stats", action="store_true",
                     help="also report cadence mean + stdev in strides/min")
    out.add_argument("--no-csv", action="store_true", help="print the report only")
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    args.pace_threshold = _parse_pace(args.pace_threshold)
    args.max_pace = _parse_pace(args.max_pace, "max pace") if args.max_pace else None
    args.min_pace = _parse_pace(args.min_pace, "min pace") if args.min_pace else None
    args.start = _parse_date(args.start, "start date") if args.start else None
    args.end = _parse_date(args.end, "end date") if args.end else None

    out_dir = Path(args.out).expanduser()
    if not out_dir.is_dir():
        print(f"error: {out_dir} is not a directory (run garmin-runs first)", file=sys.stderr)
        return 1

    activities, skipped = load_activities(out_dir)
    if not activities:
        print("No activities with trackpoints found.")
        return 0

    wanted = None if args.type == "all" else {t.strip() for t in args.type.split(",") if t.strip()}
    type_map = activity_types(out_dir) if wanted else {}
    if wanted and not type_map:
        print(f"warning: no summary.csv in {out_dir}, activity types cannot be checked", file=sys.stderr)
    for activity in activities:
        activity.type = type_map.get(activity.activity_id, "")
    activities, type_skipped, type_unknown = filter_activities(activities, wanted)
    if not activities:
        print(f"No activities left after the --type filter ({type_skipped} skipped, {type_unknown} unknown).")
        return 0

    segments: list[Segment] = []
    for activity in activities:
        if not in_range(activity, args):
            continue
        if args.method == "lap":
            segments.extend(lap_segments(activity, args))
        else:
            segments.extend(build_segments(activity, args))

    segments, dropped_slow, dropped_fast = filter_segments(segments, args)

    if not segments:
        print("No segments matched — try a smaller --window or a wider date range.")
        return 0

    notes = []
    if wanted:
        parts = []
        if type_skipped:
            parts.append(f"{type_skipped} skipped")
        if type_unknown:
            parts.append(f"{type_unknown} skipped, type unknown")
        note = f"Types: {', '.join(sorted(wanted))}"
        if parts:
            note += f" ({', '.join(parts)})"
        notes.append(note)
    if args.max_pace:
        notes.append(f"Max pace: slower than {fmt_pace(args.max_pace)}/km excluded ({dropped_slow} segments)")
    if args.min_pace:
        notes.append(f"Min pace: faster than {fmt_pace(args.min_pace)}/km excluded ({dropped_fast} segments)")
    if args.cadence_stats:
        notes.append("Cadence columns are mean + stdev in strides/min")

    print_report(activities, segments, args, skipped, notes)
    if not args.no_csv:
        write_csvs(Path(args.report_dir).expanduser(), activities, segments, args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
