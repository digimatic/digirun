from argparse import Namespace
from datetime import date, datetime, timedelta, timezone
from pathlib import Path

from garmin_runs.analyze import (
    Point,
    Activity,
    activity_types,
    build_segments,
    classify,
    filter_activities,
    filter_segments,
    in_range,
    read_gpx,
    stats,
    write_csvs,
)

FIXTURE = """<?xml version="1.0" encoding="UTF-8"?>
<gpx xmlns="http://www.topografix.com/GPX/1/1"
     xmlns:ns3="http://www.garmin.com/xmlschemas/TrackPointExtension/v1" version="1.1">
  <trk>
    <name>Test walk-run</name>
    <trkseg>
      <trkpt lat="59.33258" lon="18.06490">
        <time>2026-08-07T14:26:09.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>50</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
      <trkpt lat="59.33266" lon="18.06490">
        <time>2026-08-07T14:26:19.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>52</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
      <trkpt lat="59.33274" lon="18.06490">
        <time>2026-08-07T14:26:29.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>49</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
      <trkpt lat="59.33282" lon="18.06490">
        <time>2026-08-07T14:26:39.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>80</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
    </trkseg>
  </trk>
</gpx>
"""


def args(**overrides) -> Namespace:
    base = dict(
        method="hybrid",
        cadence_threshold=65,
        pace_threshold=540,
        window=30,
        min_segment=20,
        min_distance=3.0,
        gap=60,
        cadence_stats=False,
        max_pace=None,
        min_pace=None,
        start=None,
        end=None,
    )
    base.update(overrides)
    return Namespace(**base)


def activity(points) -> Activity:
    return Activity("1", "Test", points[0].time.date(), points, [])


def test_read_gpx_reads_points_and_cadence(tmp_path):
    path = tmp_path / "2026-08-07_1426_test_1.gpx"
    path.write_text(FIXTURE, encoding="utf-8")
    points, name = read_gpx(path)

    assert name == "Test walk-run"
    assert len(points) == 4
    assert [p.cadence for p in points] == [50, 52, 49, 80]
    # 0.00008 deg of latitude is about 8.9 m per step, so distance accumulates.
    assert points[0].distance_m == 0.0
    assert points[1].distance_m > 8
    assert points[3].distance_m > points[2].distance_m


def test_build_segments_splits_walk_and_run():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    points = [
        Point(t0, 59.33258, 18.06490, 0.0, None),
        Point(t0.replace(minute=26, second=19), 0, 0, 14.0, 50),
        Point(t0.replace(minute=26, second=29), 0, 0, 28.0, 52),
        Point(t0.replace(minute=26, second=39), 0, 0, 42.0, 49),
        Point(t0.replace(second=49), 0, 0, 70.0, 80),
        Point(t0.replace(second=59), 0, 0, 98.0, 81),
        Point(t0.replace(minute=27, second=9), 0, 0, 126.0, 82),
    ]
    segments = build_segments(activity(points), args())

    assert [s.klass for s in segments] == ["walk", "run"]
    assert segments[0].distance_m == 42.0
    assert round(segments[0].pace_s_per_km, 1) == 714.3  # 30 s / 0.042 km
    assert segments[1].distance_m == 84.0
    assert round(segments[1].pace_s_per_km, 1) == 357.1  # 30 s / 0.084 km


def test_pause_longer_than_gap_ends_the_segment():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    points = [
        Point(t0, 0, 0, 0.0, 50),
        Point(t0.replace(second=29), 0, 0, 28.0, 51),      # 20 s window, not emitted
        Point(t0.replace(minute=28, second=59), 0, 0, 28.0, 50),  # 150 s pause
        Point(t0.replace(minute=29, second=29), 0, 0, 112.0, 80),
    ]
    segments = build_segments(activity(points), args())

    # The pre-pause walking is dropped; only the post-pause run survives.
    assert [s.klass for s in segments] == ["run"]
    assert segments[0].distance_m == 84.0


def test_hybrid_resolves_the_ambiguous_cadence_band_with_pace():
    a = args()
    assert classify(500, 55, a) == "walk"
    assert classify(700, 80, a) == "run"
    assert classify(500, 65, a) == "run"   # in the band, fast enough
    assert classify(600, 65, a) == "walk"  # in the band, too slow
    assert classify(500, None, a) == "run"  # no cadence: pace decides


def test_pace_and_cadence_methods():
    pace_only = args(method="pace")
    assert classify(539, 50, pace_only) == "run"
    assert classify(541, 90, pace_only) == "walk"

    cadence_only = args(method="cadence")
    assert classify(700, 65, cadence_only) == "run"
    assert classify(400, 64, cadence_only) == "walk"
    assert classify(400, None, cadence_only) == "run"  # fallback to pace


def test_stats_handles_a_single_segment():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    single = [Point(t0, 0, 0, 0.0, None), Point(t0.replace(second=39), 0, 0, 42.0, 50)]
    segments = build_segments(activity(single), args())
    st = stats(segments)

    assert st["segments"] == 1
    assert st["stdev"] == 0.0
    assert st["min"] == st["max"] == st["mean"]
    assert st["cad_mean"] == 50.0
    assert st["cad_stdev"] is None  # stdev needs at least two segments


def test_stats_reports_cadence_mean_and_stdev():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    # Two walk windows (cadence 50 then 56) and two run windows (80 then 84).
    points = [Point(t0, 0, 0, 0.0, None)]
    for step, cadence in enumerate([50, 50, 50, 56, 56, 56], start=1):
        points.append(Point(t0 + timedelta(seconds=10 * step), 0, 0, 14.0 * step, cadence))
    for step, cadence in enumerate([80, 80, 80, 84, 84, 84], start=7):
        points.append(Point(t0 + timedelta(seconds=10 * step), 0, 0, 84.0 + 28.0 * (step - 6), cadence))

    segments = build_segments(activity(points), args())
    assert [s.klass for s in segments] == ["walk", "walk", "run", "run"]

    walk = stats(segments[:2])
    run = stats(segments[2:])
    assert walk["cad_mean"] == 53.0
    assert walk["cad_stdev"] == 4.2
    assert run["cad_mean"] == 82.0
    assert run["cad_stdev"] == 2.8


def test_analysis_csv_cadence_columns_are_optional(tmp_path):
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    points = [Point(t0, 0, 0, 0.0, None), Point(t0 + timedelta(seconds=30), 0, 0, 42.0, 50)]
    act = activity(points)
    segments = build_segments(act, args())

    write_csvs(tmp_path, [act], segments, args(cadence_stats=True))
    header = (tmp_path / "analysis.csv").read_text(encoding="utf-8").splitlines()[0]
    assert "mean_cadence" in header and "stdev_cadence" in header

    write_csvs(tmp_path, [act], segments, args())
    header = (tmp_path / "analysis.csv").read_text(encoding="utf-8").splitlines()[0]
    assert "mean_cadence" not in header


def test_max_pace_excludes_slow_segments():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    points = [
        Point(t0, 0, 0, 0.0, None),
        Point(t0 + timedelta(seconds=30), 0, 0, 42.0, 50),  # 714 s/km
        Point(t0 + timedelta(seconds=60), 0, 0, 63.0, 50),  # 1429 s/km, standing about
    ]
    segments = build_segments(activity(points), args())
    assert len(segments) == 2

    kept, slow, fast = filter_segments(segments, args(max_pace=900))
    assert slow == 1 and fast == 0
    assert len(kept) == 1 and round(kept[0].pace_s_per_km, 1) == 714.3

    kept, slow, fast = filter_segments(segments, args())
    assert slow == 0 and fast == 0 and len(kept) == 2


def test_min_pace_excludes_impossible_segments():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    points = [
        Point(t0, 0, 0, 0.0, None),
        Point(t0 + timedelta(seconds=30), 0, 0, 42.0, 50),    # 714 s/km, walking
        Point(t0 + timedelta(seconds=60), 0, 0, 532.0, 50),   # 61 s/km, GPS glitch
    ]
    segments = build_segments(activity(points), args())
    assert len(segments) == 2

    kept, slow, fast = filter_segments(segments, args(min_pace=180))
    assert fast == 1 and slow == 0
    assert len(kept) == 1 and round(kept[0].pace_s_per_km, 1) == 714.3


def test_activity_types_from_summary_csv(tmp_path):
    (tmp_path / "summary.csv").write_text(
        "date,activity_id,name,type\n2026-08-07,1,Run,running\n2026-08-07,2,Ride,cycling\n",
        encoding="utf-8",
    )
    assert activity_types(tmp_path) == {"1": "running", "2": "cycling"}
    assert activity_types(tmp_path / "missing") == {}


def test_type_filter_keeps_requested_types():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    points = [Point(t0, 0, 0, 0.0, None)]
    activities = [
        Activity("1", "Run", t0.date(), points, [], "running"),
        Activity("2", "Ride", t0.date(), points, [], "cycling"),
        Activity("3", "Walk", t0.date(), points, [], "walking"),
        Activity("4", "Mystery", t0.date(), points, [], ""),
    ]

    kept, skipped, unknown = filter_activities(activities, {"running", "trail", "walking"})
    assert [a.activity_id for a in kept] == ["1", "3"]
    assert skipped == 1 and unknown == 1

    kept, skipped, unknown = filter_activities(activities, None)
    assert len(kept) == 4 and skipped == 0 and unknown == 0


def test_in_range_compares_dates():
    t0 = datetime(2026, 8, 7, 14, 26, 9, tzinfo=timezone.utc)
    act = activity([Point(t0, 0, 0, 0.0, None)])

    assert in_range(act, args())
    assert in_range(act, args(start=date(2026, 1, 1)))
    assert not in_range(act, args(start=date(2026, 9, 1)))
    assert not in_range(act, args(end=date(2026, 6, 30)))
