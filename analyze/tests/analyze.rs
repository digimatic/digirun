//! Tests mirroring `tests/test_analyze.py`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use garmin_analyze::analyze::{
    build_segments, classify, filter_activities, filter_segments, in_range, stats,
};
use garmin_analyze::args::{Args, Method, Tables, help, parse, parse_with_today};
use garmin_analyze::csvout::write_csvs;
use garmin_analyze::dt::{Date, DateTime};
use garmin_analyze::html::render;
use garmin_analyze::load::{activity_types, load_activities};
use garmin_analyze::map::Frame;
use garmin_analyze::metrics::{
    acute_chronic, build, hr_zone, minetti_cost, prediction_source, previous_weeks, riegel, trimp,
    vdot,
};
use garmin_analyze::model::{Activity, Lap, Point, Segment, haversine};
use garmin_analyze::naming::{Change, ask, recent_routes, rewrite};
use garmin_analyze::num::round;
use garmin_analyze::parse::{read_gpx, read_tcx};
use garmin_analyze::passes::{Source, build_passes, climbs};
use garmin_analyze::routes::{
    Naming, assign_routes, describe_route, name_routes, parse_route_names,
};
use garmin_analyze::serve::{Request, State, decode_form, handle};
use garmin_analyze::xml;

const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
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
"#;

/// A timestamp with the UTC offset it carries, as in the fixture.
fn time(hour: u32, minute: u32, second: u32) -> DateTime {
    DateTime {
        date: Date {
            year: 2026,
            month: 8,
            day: 7,
        },
        hour,
        minute,
        second,
        micro: 0,
        offset: 0,
    }
}

/// A point `seconds` after 14:26:00 on 2026-08-07, normalised into a valid time.
fn point(seconds: i64, distance_m: f64, cadence: Option<f64>) -> Point {
    let total = 14 * 3600 + 26 * 60 + seconds;
    Point {
        time: time(
            (total / 3600) as u32,
            ((total % 3600) / 60) as u32,
            (total % 60) as u32,
        ),
        lat: 0.0,
        lon: 0.0,
        distance_m,
        cadence,
        hr: None,
        altitude: None,
    }
}

fn activity(points: &[Point]) -> Activity {
    Activity {
        activity_id: "1".to_string(),
        name: "Test".to_string(),
        date: points[0].time.date,
        points: points.to_vec(),
        laps: Vec::new(),
        r#type: String::new(),
    }
}

/// A unique directory under the system temp dir, cleaned up by the OS.
fn temp_dir(label: &str) -> PathBuf {
    let unique = std::process::id().to_string() + "-" + label;
    let path = std::env::temp_dir()
        .join("garmin-analyze-tests")
        .join(unique);
    std::fs::create_dir_all(&path).expect("could not create the test directory");
    path
}

#[test]
fn read_gpx_reads_points_and_cadence() {
    let dir = temp_dir("read_gpx");
    let path = dir.join("2026-08-07_1426_test_1.gpx");
    std::fs::write(&path, FIXTURE).expect("could not write the fixture");

    let content = std::fs::read_to_string(&path).expect("could not read the fixture");
    let root = xml::parse(&content).expect("could not parse the fixture");
    let (points, name) = read_gpx(&root);

    assert_eq!(name, "Test walk-run");
    assert_eq!(points.len(), 4);
    assert_eq!(
        points.iter().map(|p| p.cadence).collect::<Vec<_>>(),
        vec![Some(50.0), Some(52.0), Some(49.0), Some(80.0)]
    );
    // 0.00008 deg of latitude is about 8.9 m per step, so distance accumulates.
    assert_eq!(points[0].distance_m, 0.0);
    assert!(points[1].distance_m > 8.0);
    assert!(points[3].distance_m > points[2].distance_m);
}

#[test]
fn build_segments_splits_walk_and_run() {
    let points = vec![
        point(9, 0.0, None),
        point(19, 14.0, Some(50.0)),
        point(29, 28.0, Some(52.0)),
        point(39, 42.0, Some(49.0)),
        point(49, 70.0, Some(80.0)),
        point(59, 98.0, Some(81.0)),
        point(69, 126.0, Some(82.0)),
    ];
    let act = activity(&points);
    let segments = build_segments(&act, &Args::default());

    assert_eq!(
        segments.iter().map(|s| s.class).collect::<Vec<_>>(),
        vec!["walk", "run"]
    );
    assert_eq!(segments[0].distance_m, 42.0);
    assert_eq!(round(segments[0].pace_s_per_km, 1), 714.3); // 30 s / 0.042 km
    assert_eq!(segments[1].distance_m, 84.0);
    assert_eq!(round(segments[1].pace_s_per_km, 1), 357.1); // 30 s / 0.084 km
}

#[test]
fn pause_longer_than_gap_ends_the_segment() {
    let points = vec![
        point(9, 0.0, Some(50.0)),
        point(29, 28.0, Some(51.0)),  // 20 s window, not emitted
        point(179, 28.0, Some(50.0)), // 150 s pause
        point(209, 112.0, Some(80.0)),
    ];
    let segments = build_segments(&activity(&points), &Args::default());

    // The pre-pause walking is dropped; only the post-pause run survives.
    assert_eq!(
        segments.iter().map(|s| s.class).collect::<Vec<_>>(),
        vec!["run"]
    );
    assert_eq!(segments[0].distance_m, 84.0);
}

#[test]
fn hybrid_resolves_the_ambiguous_cadence_band_with_pace() {
    let args = Args::default();
    assert_eq!(classify(500.0, Some(55.0), &args), "walk");
    assert_eq!(classify(700.0, Some(80.0), &args), "run");
    assert_eq!(classify(500.0, Some(65.0), &args), "run"); // in the band, fast enough
    assert_eq!(classify(600.0, Some(65.0), &args), "walk"); // in the band, too slow
    assert_eq!(classify(500.0, None, &args), "run"); // no cadence: pace decides
}

#[test]
fn pace_and_cadence_methods() {
    let mut pace_only = Args::default();
    pace_only.method = Method::Pace;
    assert_eq!(classify(539.0, Some(50.0), &pace_only), "run");
    assert_eq!(classify(541.0, Some(90.0), &pace_only), "walk");

    let mut cadence_only = Args::default();
    cadence_only.method = Method::Cadence;
    assert_eq!(classify(700.0, Some(65.0), &cadence_only), "run");
    assert_eq!(classify(400.0, Some(64.0), &cadence_only), "walk");
    assert_eq!(classify(400.0, None, &cadence_only), "run"); // fallback to pace
}

#[test]
fn stats_handles_a_single_segment() {
    let points = vec![point(9, 0.0, None), point(39, 42.0, Some(50.0))];
    let act = activity(&points);
    let segments = build_segments(&act, &Args::default());
    let refs: Vec<&Segment> = segments.iter().collect();
    let st = stats(&refs);

    assert_eq!(st.segments, 1);
    assert_eq!(st.stdev, 0.0);
    assert_eq!(st.min, st.max);
    assert_eq!(st.mean, st.min);
    assert_eq!(st.cad_mean, Some(50.0));
    assert_eq!(st.cad_stdev, None); // stdev needs at least two segments
}

#[test]
fn stats_reports_cadence_mean_and_stdev() {
    // Two walk windows (cadence 50 then 56) and two run windows (80 then 84).
    let mut points = vec![point(9, 0.0, None)];
    for (step, cadence) in [50.0, 50.0, 50.0, 56.0, 56.0, 56.0].iter().enumerate() {
        let step = step + 1;
        points.push(point(
            9 + 10 * step as i64,
            14.0 * step as f64,
            Some(*cadence),
        ));
    }
    for (step, cadence) in [80.0, 80.0, 80.0, 84.0, 84.0, 84.0].iter().enumerate() {
        let step = step + 7;
        points.push(point(
            9 + 10 * step as i64,
            84.0 + 28.0 * (step - 6) as f64,
            Some(*cadence),
        ));
    }

    let act = activity(&points);
    let segments = build_segments(&act, &Args::default());
    assert_eq!(
        segments.iter().map(|s| s.class).collect::<Vec<_>>(),
        vec!["walk", "walk", "run", "run"]
    );

    let walk: Vec<&Segment> = segments.iter().take(2).collect();
    let run: Vec<&Segment> = segments.iter().skip(2).collect();
    assert_eq!(stats(&walk).cad_mean, Some(53.0));
    assert_eq!(stats(&walk).cad_stdev, Some(4.2));
    assert_eq!(stats(&run).cad_mean, Some(82.0));
    assert_eq!(stats(&run).cad_stdev, Some(2.8));
}

#[test]
fn analysis_csv_cadence_columns_are_optional() {
    let points = vec![point(9, 0.0, None), point(39, 42.0, Some(50.0))];
    let act = activity(&points);
    let segments = build_segments(&act, &Args::default());
    let refs: Vec<&Segment> = segments.iter().collect();

    let dir = temp_dir("csv_cadence");
    let with_cadence = Args {
        cadence_stats: true,
        report_dir: dir.clone(),
        ..Args::default()
    };
    write_csvs(&dir, &[&act], &refs, &with_cadence).expect("could not write the CSVs");
    let header = first_line(&dir.join("analysis.csv"));
    assert!(header.contains("mean_cadence") && header.contains("stdev_cadence"));

    let without_cadence = Args {
        report_dir: dir.clone(),
        ..Args::default()
    };
    write_csvs(&dir, &[&act], &refs, &without_cadence).expect("could not write the CSVs");
    let header = first_line(&dir.join("analysis.csv"));
    assert!(!header.contains("mean_cadence"));
}

fn first_line(path: &Path) -> String {
    std::fs::read_to_string(path)
        .expect("could not read the CSV")
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn max_pace_excludes_slow_segments() {
    let points = vec![
        point(9, 0.0, None),
        point(39, 42.0, Some(50.0)), // 714 s/km
        point(69, 63.0, Some(50.0)), // 1429 s/km, standing about
    ];
    let segments = build_segments(&activity(&points), &Args::default());
    assert_eq!(segments.len(), 2);
    let refs: Vec<&Segment> = segments.iter().collect();

    let max_pace = Args {
        max_pace: Some(900),
        ..Args::default()
    };
    let (kept, slow, fast) = filter_segments(&refs, &max_pace);
    assert_eq!((slow, fast), (1, 0));
    assert_eq!(kept.len(), 1);
    assert_eq!(round(kept[0].pace_s_per_km, 1), 714.3);

    let (kept, slow, fast) = filter_segments(&refs, &Args::default());
    assert_eq!((slow, fast), (0, 0));
    assert_eq!(kept.len(), 2);
}

#[test]
fn min_pace_excludes_impossible_segments() {
    let points = vec![
        point(9, 0.0, None),
        point(39, 42.0, Some(50.0)),  // 714 s/km, walking
        point(69, 532.0, Some(50.0)), // 61 s/km, GPS glitch
    ];
    let segments = build_segments(&activity(&points), &Args::default());
    assert_eq!(segments.len(), 2);
    let refs: Vec<&Segment> = segments.iter().collect();

    let min_pace = Args {
        min_pace: Some(180),
        ..Args::default()
    };
    let (kept, slow, fast) = filter_segments(&refs, &min_pace);
    assert_eq!((fast, slow), (1, 0));
    assert_eq!(kept.len(), 1);
    assert_eq!(round(kept[0].pace_s_per_km, 1), 714.3);
}

#[test]
fn activity_types_from_summary_csv() {
    let dir = temp_dir("summary");
    std::fs::write(
        dir.join("summary.csv"),
        "date,activity_id,name,type\n2026-08-07,1,Run,running\n2026-08-07,2,Ride,cycling\n",
    )
    .expect("could not write summary.csv");

    let mut expected = BTreeMap::new();
    expected.insert("1".to_string(), "running".to_string());
    expected.insert("2".to_string(), "cycling".to_string());
    assert_eq!(activity_types(&dir), expected);
    assert_eq!(activity_types(&dir.join("missing")), BTreeMap::new());
}

#[test]
fn type_filter_keeps_requested_types() {
    let points = vec![point(9, 0.0, None)];
    let activities: Vec<Activity> = ["1", "2", "3", "4"]
        .iter()
        .zip(["Run", "Ride", "Walk", "Mystery"].iter())
        .zip(["running", "cycling", "walking", ""].iter())
        .map(|((id, name), type_name)| Activity {
            activity_id: id.to_string(),
            name: name.to_string(),
            date: points[0].time.date,
            points: points.clone(),
            laps: Vec::new(),
            r#type: type_name.to_string(),
        })
        .collect();
    let refs: Vec<&Activity> = activities.iter().collect();

    let wanted: BTreeSet<String> = ["running", "trail", "walking"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    let (kept, skipped, unknown) = filter_activities(&refs, &Some(wanted));
    assert_eq!(
        kept.iter()
            .map(|a| a.activity_id.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "3"]
    );
    assert_eq!((skipped, unknown), (1, 1));

    let (kept, skipped, unknown) = filter_activities(&refs, &None);
    assert_eq!((kept.len(), skipped, unknown), (4, 0, 0));
}

#[test]
fn in_range_compares_dates() {
    let points = vec![point(9, 0.0, None)];
    let act = activity(&points);

    assert!(in_range(&act, &Args::default()));
    assert!(in_range(
        &act,
        &Args {
            start: Some(Date {
                year: 2026,
                month: 1,
                day: 1
            }),
            ..Args::default()
        }
    ));
    assert!(!in_range(
        &act,
        &Args {
            start: Some(Date {
                year: 2026,
                month: 9,
                day: 1
            }),
            ..Args::default()
        }
    ));
    assert!(!in_range(
        &act,
        &Args {
            end: Some(Date {
                year: 2026,
                month: 6,
                day: 30
            }),
            ..Args::default()
        }
    ));
}

#[test]
fn load_activities_is_the_same_with_and_without_the_pool() {
    // enough files to cross MIN_PARALLEL_FILES, so the pool path is exercised
    let dir = temp_dir("pool");
    for i in 0..10 {
        std::fs::write(dir.join(format!("2026-08-07_{}.gpx", 1000 + i)), FIXTURE)
            .expect("could not write the fixture");
    }

    let serial = load_activities(&dir, 1);
    let parallel = load_activities(&dir, 4);

    assert_eq!(parallel.activities, serial.activities);
    assert_eq!(parallel.skipped, serial.skipped);
    assert_eq!(serial.activities.len(), 10);
}

/// A 1 Hz track from `(seconds, cadence, metres per second)` stretches.
fn track(stretches: &[(i64, f64, f64)]) -> Vec<Point> {
    let mut points = vec![point(0, 0.0, None)];
    let (mut t, mut distance) = (0, 0.0);
    for &(seconds, cadence, speed) in stretches {
        for _ in 0..seconds {
            t += 1;
            distance += speed;
            points.push(point(t, distance, Some(cadence)));
        }
    }
    points
}

fn with_one_lap(points: &[Point]) -> Activity {
    let mut act = activity(points);
    act.laps = vec![Lap {
        start_index: 0,
        end_index: points.len() - 1,
        duration_s: (points.len() - 1) as f64,
        distance_m: points[points.len() - 1].distance_m,
    }];
    act
}

#[test]
fn passes_split_on_cadence_within_one_lap() {
    let points = track(&[
        (60, 54.0, 1.3),
        (120, 78.0, 2.5),
        (40, 54.0, 1.3),
        (60, 78.0, 2.5),
    ]);
    let passes = build_passes(&with_one_lap(&points), &Args::default());

    assert_eq!(
        passes.iter().map(|p| p.class).collect::<Vec<_>>(),
        vec!["walk", "run", "walk", "run"]
    );
    // The median shifts each boundary by at most half the smoothing width.
    for (pass, want) in passes.iter().zip([60.0, 120.0, 40.0, 60.0]) {
        assert!((pass.stats.time_s - want).abs() <= 5.0, "{pass:?}");
        assert_eq!(pass.laps, vec![1]);
        assert_eq!(pass.source, Source::Cadence);
    }
    let total: f64 = passes.iter().map(|p| p.stats.time_s).sum();
    assert_eq!(total, 280.0);
}

#[test]
fn cadence_inside_the_band_keeps_the_pass() {
    // 65 is inside the 61-71 band: it stays a run even at a walking pace.
    let points = track(&[(60, 78.0, 2.5), (60, 65.0, 1.3)]);
    let passes = build_passes(&activity(&points), &Args::default());

    assert_eq!(
        passes.iter().map(|p| p.class).collect::<Vec<_>>(),
        vec!["run"]
    );
    assert_eq!(passes[0].stats.time_s, 120.0);
}

#[test]
fn short_passes_join_a_neighbour() {
    let points = track(&[(60, 78.0, 2.5), (10, 54.0, 1.3), (60, 78.0, 2.5)]);
    let args = Args {
        smooth: 0,
        ..Args::default()
    };
    let passes = build_passes(&activity(&points), &args);
    assert_eq!(
        passes.iter().map(|p| p.class).collect::<Vec<_>>(),
        vec!["run"]
    );
    assert_eq!(passes[0].stats.time_s, 130.0);

    let args = Args {
        smooth: 0,
        min_pass: 5,
        ..Args::default()
    };
    let passes = build_passes(&activity(&points), &args);
    assert_eq!(
        passes.iter().map(|p| p.class).collect::<Vec<_>>(),
        vec!["run", "walk", "run"]
    );
}

#[test]
fn pause_longer_than_gap_splits_the_pass() {
    let mut points = track(&[(60, 78.0, 2.5)]);
    let resume = track(&[(60, 78.0, 2.5)]);
    let last = points[points.len() - 1].distance_m;
    for p in &resume[1..] {
        let seconds = 60 + 120 + p.time.seconds_since(&resume[0].time) as i64;
        points.push(point(seconds, last + p.distance_m, p.cadence));
    }
    let passes = build_passes(&activity(&points), &Args::default());

    assert_eq!(
        passes.iter().map(|p| p.class).collect::<Vec<_>>(),
        vec!["run", "run"]
    );
    // The first step after the pause spans the gap, so it is not counted.
    assert_eq!(passes[0].stats.time_s, 60.0);
    assert_eq!(passes[1].stats.time_s, 59.0);
}

#[test]
fn pass_stats_cover_hr_elevation_and_stride() {
    let mut points = track(&[(120, 80.0, 2.5)]);
    for (i, p) in points.iter_mut().enumerate() {
        p.hr = Some(120.0 + i as f64 / 4.0); // 120 .. 150
        p.altitude = Some(if i <= 60 { i as f64 } else { 120.0 - i as f64 });
    }
    let passes = build_passes(&activity(&points), &Args::default());
    let s = &passes[0].stats;

    assert_eq!(passes.len(), 1);
    assert_eq!(s.time_s, 120.0);
    assert_eq!(s.distance_m, 300.0);
    assert_eq!(s.pace, Some(400.0)); // 120 s / 0.3 km
    assert_eq!(s.pace_sd, Some(0.0));
    assert_eq!(
        (s.cad_mean, s.cad_min, s.cad_max),
        (Some(80.0), Some(80.0), Some(80.0))
    );
    assert_eq!((s.hr_min, s.hr_max), (Some(120.2), Some(150.0))); // 120.25, rounded
    assert_eq!(s.hr_drift, None); // two minutes is mostly HR lag
    // The rolling mean rounds off the 60 m peak and the track's ends.
    let (gain, loss) = (s.elev_gain.unwrap(), s.elev_loss.unwrap());
    assert!((40.0..60.0).contains(&gain), "gain {gain}");
    assert!((40.0..60.0).contains(&loss), "loss {loss}");
    assert_eq!(s.stride_m, Some(1.88)); // 300 m / (80 strides/min * 2 min)
}

#[test]
fn read_tcx_reads_hr_and_altitude() {
    let tcx = r#"<?xml version="1.0" encoding="UTF-8"?>
<TrainingCenterDatabase xmlns="http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2"
    xmlns:ns3="http://www.garmin.com/xmlschemas/ActivityExtension/v2">
  <Activities><Activity Sport="Running"><Lap StartTime="2026-08-07T14:26:00.000Z">
    <TotalTimeSeconds>1.0</TotalTimeSeconds><DistanceMeters>3.0</DistanceMeters>
    <Track>
      <Trackpoint><Time>2026-08-07T14:26:00.000Z</Time><AltitudeMeters>128.5</AltitudeMeters>
        <DistanceMeters>0.0</DistanceMeters><HeartRateBpm><Value>94</Value></HeartRateBpm>
        <Extensions><ns3:TPX><ns3:RunCadence>54</ns3:RunCadence></ns3:TPX></Extensions></Trackpoint>
      <Trackpoint><Time>2026-08-07T14:26:01.000Z</Time>
        <DistanceMeters>3.0</DistanceMeters></Trackpoint>
    </Track>
  </Lap></Activity></Activities>
</TrainingCenterDatabase>"#;
    let root = xml::parse(tcx).expect("could not parse the fixture");
    let (points, laps) = read_tcx(&root);

    assert_eq!(laps.len(), 1);
    assert_eq!(
        (points[0].hr, points[0].altitude, points[0].cadence),
        (Some(94.0), Some(128.5), Some(54.0))
    );
    assert_eq!((points[1].hr, points[1].altitude), (None, None));
}

#[test]
fn read_gpx_reads_hr_and_elevation() {
    let gpx = FIXTURE.replace(
        "<ns3:cad>50</ns3:cad>",
        "<ns3:hr>101</ns3:hr><ns3:cad>50</ns3:cad>",
    );
    let gpx = gpx.replacen("<time>", "<ele>12.5</ele><time>", 1);
    let root = xml::parse(&gpx).expect("could not parse the fixture");
    let (points, _) = read_gpx(&root);

    assert_eq!(
        (points[0].hr, points[0].altitude),
        (Some(101.0), Some(12.5))
    );
    assert_eq!((points[1].hr, points[1].altitude), (None, None));
}

#[test]
fn days_counts_back_from_end_or_today() {
    let argv = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let today = Date {
        year: 2026,
        month: 1,
        day: 3,
    };

    let args = parse_with_today(&argv(&["--days", "7"]), today).expect("--days should parse");
    assert_eq!(
        (args.start, args.end, args.days),
        (Date::from_iso("2025-12-28"), None, Some(7))
    );
    let args = parse_with_today(&argv(&["--end", "2026-10-18", "--days=7"]), today)
        .expect("--days with --end should parse");
    assert_eq!(args.start, Date::from_iso("2026-10-12"));

    for bad in [
        &["--days", "0"][..],
        &["--days", "3", "--start", "2026-10-01"][..],
    ] {
        let error = parse_with_today(&argv(bad), today).expect_err("should be refused");
        assert_eq!(error.code, 2);
    }
    // No range at all: every table covers all of time.
    assert_eq!(parse_with_today(&[], today).expect("empty").start, None);
}

#[test]
fn detail_options_parse() {
    let argv: Vec<String> = [
        "--show",
        "detail",
        "--activity",
        "1, 2",
        "--min-pass=30",
        "--smooth",
        "6",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let args = parse(&argv).expect("the options should parse");

    assert!(args.show.detail);
    assert!(!args.show.segments());
    assert_eq!(
        args.activities,
        Some(BTreeSet::from(["1".to_string(), "2".to_string()]))
    );
    assert_eq!((args.min_pass, args.smooth), (30, 6));
    // --detail was folded into --show.
    assert_eq!(parse(&["--detail".to_string()]).expect_err("gone").code, 2);
}

#[test]
fn stats_mean_pace_is_time_over_distance() {
    let fast = Segment {
        activity_id: "1".to_string(),
        date: time(14, 26, 0).date,
        time: time(14, 26, 30),
        duration_s: 30.0,
        distance_m: 100.0, // 5:00/km
        pace_s_per_km: 300.0,
        cadence: None,
        class: "run",
    };
    let slow = Segment {
        distance_m: 50.0, // 10:00/km
        pace_s_per_km: 600.0,
        ..fast.clone()
    };
    // 60 s over 150 m, not the 7:30 average of the two paces.
    assert_eq!(stats(&[&fast, &slow]).mean, 400.0);
}

#[test]
fn standing_about_inside_a_pass_is_left_out() {
    // 60 s walking, 30 s standing at walking cadence, 60 s walking.
    let points = track(&[(60, 54.0, 1.3), (30, 54.0, 0.05), (60, 54.0, 1.3)]);
    let args = Args {
        max_pace: Some(900),
        ..Args::default()
    };
    let passes = build_passes(&activity(&points), &args);

    assert_eq!(passes.len(), 1);
    let s = &passes[0].stats;
    assert_eq!((s.time_s, s.stopped_s), (120.0, 30.0));
    assert_eq!(s.distance_m, 156.0);
    assert!(!passes[0].excluded);
}

#[test]
fn show_picks_the_report_tables() {
    let argv = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    let args = parse(&argv(&["--show", "weekly, overall"])).expect("the option should parse");
    assert_eq!(
        args.show,
        Tables {
            activities: false,
            weekly: true,
            overall: true,
            ..Tables::none()
        }
    );
    // The default is the segment tables; 'all' adds the others.
    assert_eq!(Args::default().show, Tables::default());
    assert!(!Tables::default().detail && Tables::default().segments());
    assert_eq!(
        parse(&argv(&["--show=all"]))
            .expect("all should parse")
            .show,
        Tables::all()
    );

    let error = parse(&argv(&["--show", "week"])).expect_err("week is not a table");
    assert_eq!(error.code, 2);
    assert!(error.message.contains("invalid choice: 'week'"));
    assert!(help().contains("--show TABLES"));
    assert!(help().contains("--days N"));
}

fn date(year: i32, month: u32, day: u32) -> Date {
    Date { year, month, day }
}

/// An activity on `on`, with HR from `hr(second)`.
fn session_on(on: Date, id: &str, stretches: &[(i64, f64, f64)], hr: fn(usize) -> f64) -> Activity {
    let mut points = track(stretches);
    for (i, p) in points.iter_mut().enumerate() {
        p.hr = Some(hr(i));
    }
    Activity {
        activity_id: id.to_string(),
        date: on,
        ..activity(&points)
    }
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

#[test]
fn isocalendar_follows_the_thursday() {
    assert_eq!(date(2026, 11, 3).isocalendar(), (2026, 45));
    assert_eq!(date(2025, 12, 29).isocalendar(), (2026, 1));
    assert_eq!(date(2026, 12, 31).isocalendar(), (2026, 53));
    assert_eq!(date(2021, 1, 3).isocalendar(), (2020, 53));
    assert_eq!(date(2026, 11, 3).monday(), date(2026, 11, 2));
}

#[test]
fn elevation_noise_does_not_add_up_to_climbing() {
    let mut points = track(&[(600, 80.0, 2.5)]);
    for (i, p) in points.iter_mut().enumerate() {
        p.altitude = Some(if i % 2 == 0 { 10.6 } else { 9.4 });
    }
    let gain: f64 = climbs(&points).iter().map(|c| c.0).sum();
    assert!(gain < 1.0, "phantom gain {gain}");
}

#[test]
fn elevation_keeps_a_real_climb() {
    let mut points = track(&[(600, 80.0, 2.5)]);
    for (i, p) in points.iter_mut().enumerate() {
        // Flat, then 30 m up over 300 s, then flat.
        p.altitude = Some((i as f64 - 150.0).clamp(0.0, 300.0) / 10.0);
    }
    let gain: f64 = climbs(&points).iter().map(|c| c.0).sum();
    near(gain, 30.0, 1.0);
}

#[test]
fn zero_cadence_is_missing_rather_than_slow() {
    // Half the window stopped: the mean is of the steps that were taken.
    let points = track(&[(15, 0.0, 1.3), (15, 50.0, 1.3)]);
    let segments = build_segments(&activity(&points), &Args::default());
    assert_eq!(segments[0].cadence, Some(50.0));
}

#[test]
fn a_stop_with_zero_cadence_splits_the_runs() {
    let points = track(&[(120, 80.0, 2.5), (60, 0.0, 0.0), (120, 80.0, 2.5)]);
    let classes: Vec<&str> = build_passes(&activity(&points), &Args::default())
        .iter()
        .map(|p| p.class)
        .collect();
    assert_eq!(classes, vec!["run", "walk", "run"]);
}

#[test]
fn a_pass_under_one_window_has_no_pace_or_stride() {
    let points = track(&[(120, 80.0, 2.5), (25, 50.0, 1.3), (120, 80.0, 2.5)]);
    let passes = build_passes(&activity(&points), &Args::default());
    assert_eq!(passes.len(), 3);
    assert!(!passes[1].stats.pace_reliable);
    assert_eq!(passes[1].stats.stride_m, None);
    assert!(passes[0].stats.pace_reliable);
    assert!(passes[0].stats.stride_m.is_some());
}

#[test]
fn hr_drift_needs_five_minutes() {
    let mut points = track(&[(600, 80.0, 2.5)]);
    for (i, p) in points.iter_mut().enumerate() {
        p.hr = Some(120.0 + i as f64 / 20.0); // 120 .. 150
    }
    let passes = build_passes(&activity(&points), &Args::default());
    near(passes[0].stats.hr_drift.unwrap(), 20.0, 0.1);
}

#[test]
fn hr_zones_and_trimp() {
    assert_eq!(hr_zone(99.0, 200.0), None);
    assert_eq!(hr_zone(100.0, 200.0), Some(0));
    assert_eq!(hr_zone(120.0, 200.0), Some(1));
    assert_eq!(hr_zone(179.0, 200.0), Some(3));
    assert_eq!(hr_zone(180.0, 200.0), Some(4));
    assert_eq!(hr_zone(210.0, 200.0), Some(4));
    assert_eq!(trimp(&[60.0, 0.0, 120.0, 0.0, 0.0]), 7.0); // 1 min * 1 + 2 min * 3
}

#[test]
fn session_zones_need_hr_max() {
    let a = session_on(date(2026, 8, 3), "1", &[(600, 80.0, 2.5)], |_| 150.0);
    let without = build(&[&a], &Args::default());
    assert_eq!(without.sessions[0].trimp, None);

    let args = Args {
        hr_max: Some(200),
        ..Args::default()
    };
    let with = build(&[&a], &args);
    assert_eq!(with.sessions[0].zones_run[2], 600.0); // 75% is zone 3
    assert_eq!(with.sessions[0].trimp, Some(30.0)); // 10 min * 3
    near(with.sessions[0].zone3_pace.unwrap(), 400.0, 0.1);
}

#[test]
fn weeks_keep_the_gaps_and_baselines_look_back() {
    let run = |on, id: &str| session_on(on, id, &[(600, 80.0, 2.5)], |_| 150.0);
    let activities = [
        run(date(2026, 8, 3), "1"),
        run(date(2026, 8, 5), "2"),
        run(date(2026, 8, 31), "3"),
        run(date(2026, 9, 7), "4"),
    ];
    let refs: Vec<&Activity> = activities.iter().collect();
    // The displayed range does not cut the history the baseline needs.
    let args = Args {
        start: Some(date(2026, 9, 1)),
        ..Args::default()
    };
    let metrics = build(&refs, &args);

    assert_eq!(metrics.weeks.len(), 6);
    assert_eq!(metrics.weeks[0].sessions, 2);
    assert_eq!(metrics.weeks[1].sessions, 0);
    assert_eq!(metrics.weeks[0].km_baseline, None);
    assert_eq!(metrics.weeks[3].km_baseline, None); // three weeks of history
    near(metrics.weeks[4].km_baseline.unwrap(), 0.75, 0.01); // 3 km over 4 weeks
    assert_eq!(metrics.weeks[5].longest_run_s, 600.0);
    assert_eq!(metrics.weeks[5].longest_run_m, 1500.0);
    assert!(!metrics.weeks[3].in_range(&args));
    assert!(metrics.weeks[4].in_range(&args)); // its Sunday is in range

    let later = previous_weeks(&metrics.weeks, date(2026, 9, 28)).unwrap();
    near(later.km, 3.0 / 4.0, 0.01); // weeks after the last session are empty
}

#[test]
fn acute_chronic_keeps_the_acute_week_out_of_the_baseline() {
    let reference = date(2026, 11, 3);
    let samples = [
        (reference.minus_days(1), 10.0),
        (reference.minus_days(10), 8.0),
        (reference.minus_days(34), 12.0),
        (reference.minus_days(35), 100.0), // too old to count
    ];
    let load = acute_chronic(&samples, reference);
    assert_eq!((load.acute, load.baseline), (10.0, 5.0));
    assert_eq!(load.ratio, Some(2.0));

    // Under 4 weeks of history there is no ratio to read.
    let load = acute_chronic(&samples[..2], reference);
    assert_eq!(load.ratio, None);
}

#[test]
fn decoupling_skips_the_warm_up() {
    // HR 100 for 5 minutes, then 150 and 165 over equal halves at a steady speed.
    let a = session_on(date(2026, 8, 3), "1", &[(1800, 80.0, 2.5)], |i| match i {
        0..=300 => 100.0,
        301..=1050 => 150.0,
        _ => 165.0,
    });
    let metrics = build(&[&a], &Args::default());
    near(
        metrics.sessions[0].decoupling.unwrap(),
        (1.0 - 150.0 / 165.0) * 100.0,
        0.1,
    );

    // Shorter than 20 minutes is not enough to judge.
    let short = session_on(date(2026, 8, 3), "2", &[(1100, 80.0, 2.5)], |_| 150.0);
    assert_eq!(
        build(&[&short], &Args::default()).sessions[0].decoupling,
        None
    );
}

#[test]
fn recovery_is_the_hr_drop_in_the_first_minute_of_a_walk() {
    let a = session_on(
        date(2026, 8, 3),
        "1",
        &[(180, 80.0, 2.5), (120, 50.0, 1.3)],
        |i| {
            if i <= 180 {
                170.0
            } else {
                170.0 - (i - 180) as f64 / 2.0
            }
        },
    );
    let metrics = build(&[&a], &Args::default());
    near(metrics.sessions[0].recovery.unwrap(), 30.0, 4.0);

    // A short run is not followed by a meaningful drop.
    let short = session_on(
        date(2026, 8, 3),
        "2",
        &[(60, 80.0, 2.5), (120, 50.0, 1.3)],
        |_| 150.0,
    );
    assert_eq!(
        build(&[&short], &Args::default()).sessions[0].recovery,
        None
    );
}

#[test]
fn routes_group_on_overlap() {
    let cells = |range: std::ops::Range<i64>| range.map(|i| (i, 0)).collect::<BTreeSet<_>>();
    let tracks = [
        cells(0..100),
        cells(0..90),    // 90% of the first
        cells(500..600), // elsewhere
        BTreeSet::new(), // no GPS
        cells(50..150),  // a third of the first
        cells(505..600),
    ];
    assert_eq!(
        assign_routes(&tracks),
        vec![Some(0), Some(0), Some(1), None, Some(2), Some(1)]
    );
}

/// A track through these offsets in metres east and north, a point every ~20 m.
fn shaped(corners: &[(f64, f64)]) -> Activity {
    let (lat0, lon0) = (57.7, 12.0);
    let metres_per_degree = 111_320.0;
    let mut offsets = vec![corners[0]];
    for pair in corners.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let steps = ((x1 - x0).hypot(y1 - y0) / 20.0).ceil() as usize;
        for k in 1..=steps {
            let f = k as f64 / steps as f64;
            offsets.push((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f));
        }
    }
    let mut points: Vec<Point> = Vec::new();
    for (i, (x, y)) in offsets.into_iter().enumerate() {
        let mut p = point(i as i64 * 5, 0.0, Some(80.0));
        p.lat = lat0 + y / metres_per_degree;
        p.lon = lon0 + x / (metres_per_degree * lat0.to_radians().cos());
        if let Some(last) = points.last() {
            p.distance_m = last.distance_m + haversine(last.lat, last.lon, p.lat, p.lon);
        }
        points.push(p);
    }
    activity(&points)
}

#[test]
fn routes_are_described_by_shape_and_direction() {
    let square = [
        (0.0, 0.0),
        (0.0, 1000.0),
        (1000.0, 1000.0),
        (1000.0, 0.0),
        (0.0, 0.0),
    ];
    assert_eq!(describe_route(0, &shaped(&square)), "R1 · 4.0 km loop NE");
    let there_and_back = [(0.0, 0.0), (0.0, 2000.0), (0.0, 0.0)];
    assert_eq!(
        describe_route(1, &shaped(&there_and_back)),
        "R2 · 4.0 km out-and-back N"
    );
    let east = [(0.0, 0.0), (3000.0, 0.0)];
    assert_eq!(describe_route(2, &shaped(&east)), "R3 · 3.0 km one-way E");
    assert_eq!(describe_route(3, &run_on("1", "Run", 3)), "R4");
}

#[test]
fn route_names_file_parses() {
    let text = "# my routes\n\n 2 = Home loop \n1=Lake # by the water\n";
    assert_eq!(
        parse_route_names(text).unwrap(),
        vec![
            ("2".to_string(), "Home loop".to_string()),
            ("1".to_string(), "Lake".to_string()),
        ]
    );
    assert_eq!(
        parse_route_names("1 = A\n2 Home\n").unwrap_err(),
        "line 2: expected `<activity id> = <name>`"
    );
    assert!(parse_route_names("= Home").is_err());
    assert!(parse_route_names("2 =").is_err());
}

#[test]
fn any_session_names_its_route() {
    let (a, b, far, untracked) = (
        run_from("1", 3, 57.74, 12.12),
        run_from("2", 4, 57.74, 12.12),
        run_from("3", 5, 40.0, -3.7),
        run_on("4", "Run", 6),
    );
    let args = html_args(None);
    let mut metrics = build(&[&a, &b, &far, &untracked], &args);
    let names =
        parse_route_names("2 = Home loop\n1 = Other\n1 = Home loop\n99 = Gone\n4 = Treadmill\n")
            .unwrap();
    assert_eq!(
        name_routes(&mut metrics, &names),
        Naming {
            unmatched: vec!["99".to_string()],
            conflicts: vec![
                "1 = Other: its route is already named Home loop".to_string(),
                "4 has no GPS track to name".to_string(),
            ],
        }
    );
    let named: Vec<&str> = metrics
        .sessions
        .iter()
        .map(|s| s.route_name.as_str())
        .collect();
    assert_eq!(named[..2], ["Home loop", "Home loop"]);
    assert!(named[2].starts_with("R2 · "), "{}", named[2]);
    assert_eq!(named[3], "no GPS");
    let page = render(&metrics, &args, august(6));
    assert!(page.contains("<div class=\"label\">Home loop</div>"));
    assert!(page.contains("title=\"Name it in routes.txt: 2 = ...\""));
}

/// Two sessions on a home route, one far away, one without GPS; home named "Home".
fn named_metrics<'a>(runs: &'a [Activity; 4]) -> garmin_analyze::metrics::Metrics<'a> {
    let refs: Vec<&Activity> = runs.iter().collect();
    let mut metrics = build(&refs, &Args::default());
    name_routes(&mut metrics, &parse_route_names("1 = Home").unwrap());
    metrics
}

fn four_runs() -> [Activity; 4] {
    [
        run_from("1", 3, 57.74, 12.12),
        run_from("3", 5, 40.0, -3.7),
        run_from("2", 20, 57.74, 12.12),
        run_on("4", "Run", 21),
    ]
}

#[test]
fn name_routes_option_asks_about_the_last_30_days() {
    let today = august(31);
    let args = parse_with_today(&["--name-routes".to_string()], today).unwrap();
    assert!(args.name_routes);
    assert_eq!(args.start, Some(august(2)));
    let days = [
        "--name-routes".to_string(),
        "--days".to_string(),
        "7".to_string(),
    ];
    assert_eq!(
        parse_with_today(&days, today).unwrap().start,
        Some(august(25))
    );
    assert!(help().contains("--name-routes"));
}

#[test]
fn recent_routes_are_the_ones_run_in_range() {
    let runs = four_runs();
    let metrics = named_metrics(&runs);
    let args = Args {
        start: Some(august(4)),
        ..Args::default()
    };
    let routes = recent_routes(&metrics, &args);
    let summary: Vec<(usize, &str, usize, &str)> = routes
        .iter()
        .map(|r| {
            (
                r.index,
                r.current.as_str(),
                r.sessions,
                r.newest_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [(0, "Home", 2, "2"), (1, routes[1].auto.as_str(), 1, "3")]
    );
    assert!(routes[0].auto.starts_with("R1 · "));
    let later = Args {
        start: Some(august(6)),
        ..Args::default()
    };
    assert_eq!(recent_routes(&metrics, &later).len(), 1);
}

#[test]
fn ask_keeps_renames_and_clears() {
    let runs = four_runs();
    let metrics = named_metrics(&runs);
    let routes = recent_routes(&metrics, &Args::default());
    let mut shown = Vec::new();
    let changes = ask(&routes, "-\nCafe # no\nCafe\n".as_bytes(), &mut shown).unwrap();
    assert_eq!(
        changes,
        [
            Change {
                route: 0,
                name: None
            },
            Change {
                route: 1,
                name: Some("Cafe".to_string())
            },
        ]
    );
    let shown = String::from_utf8(shown).unwrap();
    assert!(shown.contains("  name [Home]: "));
    assert!(shown.contains("can't contain #"));
    // Enter keeps; clearing an unnamed route, or end of input, changes nothing.
    assert!(
        ask(&routes, "\n-\n".as_bytes(), Vec::new())
            .unwrap()
            .is_empty()
    );
    assert!(ask(&routes, "".as_bytes(), Vec::new()).unwrap().is_empty());
    assert_eq!(
        ask(&routes, "Park\n".as_bytes(), Vec::new()).unwrap().len(),
        1
    );
}

#[test]
fn rewrite_replaces_the_lines_of_changed_routes() {
    let runs = four_runs();
    let metrics = named_metrics(&runs);
    let routes = recent_routes(&metrics, &Args::default());
    let text = "# mine\n1 = Home\n77 = Elsewhere\n";
    let renamed = [
        Change {
            route: 0,
            name: Some("Lake".to_string()),
        },
        Change {
            route: 1,
            name: Some("Madrid".to_string()),
        },
    ];
    assert_eq!(
        rewrite(text, &renamed, &metrics, &routes),
        "# mine\n77 = Elsewhere\n2 = Lake\n3 = Madrid\n"
    );
    let cleared = [Change {
        route: 0,
        name: None,
    }];
    assert_eq!(
        rewrite(text, &cleared, &metrics, &routes),
        "# mine\n77 = Elsewhere\n"
    );
    assert!(rewrite("", &renamed, &metrics, &routes).starts_with("# Route names"));
}

#[test]
fn minetti_cost_of_grades() {
    near(minetti_cost(0.0), 3.6, 1e-9);
    near(minetti_cost(0.10), 5.968, 0.001);
    near(minetti_cost(-0.10), 2.152, 0.001);
    assert_eq!(minetti_cost(0.5), minetti_cost(0.3)); // clamped
}

#[test]
fn grade_adjusted_pace_is_faster_uphill() {
    let mut points = track(&[(600, 80.0, 2.5)]);
    for (i, p) in points.iter_mut().enumerate() {
        p.altitude = Some(i as f64 * 0.25); // a steady 10% grade
    }
    let a = activity(&points);
    let s = &build(&[&a], &Args::default()).sessions[0];
    near(s.run_pace().unwrap(), 400.0, 0.1);
    near(s.grade_adjusted_pace.unwrap(), 400.0 / (5.968 / 3.6), 3.0);
}

#[test]
fn best_effort_finds_the_fastest_stretch() {
    let a = session_on(
        date(2026, 8, 3),
        "1",
        &[(600, 80.0, 2.0), (300, 80.0, 4.0), (600, 80.0, 2.0)],
        |_| 150.0,
    );
    let free = build(&[&a], &Args::default());
    near(
        free.sessions[0].efforts_any[0].as_ref().unwrap().time_s,
        250.0,
        1.0,
    );

    // A pace faster than --min-pace is taken for a GPS glitch.
    let args = Args {
        min_pace: Some(300),
        ..Args::default()
    };
    let guarded = build(&[&a], &args);
    let time = guarded.sessions[0].efforts_any[0].as_ref().unwrap().time_s;
    assert!(time >= 300.0, "{time} beats the guard");
    near(time, 300.0, 2.0);
}

#[test]
fn running_efforts_leave_out_walk_runs() {
    let a = session_on(
        date(2026, 8, 3),
        "1",
        &[(267, 80.0, 3.0), (1000, 50.0, 1.5)],
        |_| 150.0,
    );
    let s = &build(&[&a], &Args::default()).sessions[0];
    assert!(s.efforts_any[0].is_some());
    assert_eq!(s.efforts_running[0], None);
}

#[test]
fn predictions_need_a_long_recent_run() {
    near(riegel(1200.0, 5000.0, 10000.0), 2502.0, 1.0);
    near(vdot(5000.0, 1200.0), 49.8, 0.1);

    let long = session_on(date(2026, 8, 3), "1", &[(1240, 80.0, 2.5)], |_| 150.0);
    let short = session_on(date(2026, 8, 3), "2", &[(600, 80.0, 2.5)], |_| 150.0);
    let metrics = build(&[&long], &Args::default());
    let source = prediction_source(&metrics.sessions, date(2026, 8, 10)).unwrap();
    assert_eq!(source.distance_m, 3000.0);
    assert_eq!(
        prediction_source(&metrics.sessions, date(2026, 11, 2)),
        None
    ); // 91 days
    let metrics = build(&[&short], &Args::default());
    assert_eq!(
        prediction_source(&metrics.sessions, date(2026, 8, 10)),
        None
    );
}

#[test]
fn metric_options_parse() {
    let argv = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    let args = parse(&argv(&["--show", "summary,zones", "--hr-max=186"])).expect("should parse");
    assert_eq!(args.hr_max, Some(186));
    assert!(args.show.summary && args.show.zones && !args.show.progress);
    assert!(args.show.metrics() && !args.show.segments());
    assert!(!Tables::default().metrics());
    assert!(Tables::all().metrics());
    assert_eq!(
        parse(&argv(&["--hr-max", "20"])).expect_err("too low").code,
        2
    );
    assert_eq!(
        parse(&argv(&["--hr-max", "x"]))
            .expect_err("not a number")
            .code,
        2
    );
    assert!(help().contains("--hr-max BPM"));
}

#[test]
fn longest_run_distance_belongs_to_the_longest_time() {
    // The slower run lasts longer but covers less ground than the faster one.
    let slow = session_on(date(2026, 8, 3), "1", &[(600, 80.0, 1.5)], |_| 150.0);
    let fast = session_on(date(2026, 8, 5), "2", &[(400, 80.0, 3.0)], |_| 150.0);
    let metrics = build(&[&slow, &fast], &Args::default());
    assert_eq!(metrics.weeks[0].longest_run_s, 600.0);
    assert_eq!(metrics.weeks[0].longest_run_m, 900.0);

    let later = previous_weeks(&metrics.weeks, date(2026, 8, 31)).unwrap();
    assert_eq!((later.longest_run_s, later.longest_run_m), (600.0, 900.0));
}

fn html_args(hr_max: Option<i64>) -> Args {
    Args {
        hr_max,
        ..Args::default()
    }
}

/// Eight minutes of running with HR on a given day of August 2026.
fn run_on(id: &str, name: &str, day: u32) -> Activity {
    let mut points = track(&[(480, 80.0, 3.0)]);
    for point in &mut points {
        point.time.date.day = day;
        point.hr = Some(150.0);
    }
    let mut activity = activity(&points);
    activity.activity_id = id.to_string();
    activity.name = name.to_string();
    activity
}

fn august(day: u32) -> Date {
    Date {
        year: 2026,
        month: 8,
        day,
    }
}

#[test]
fn html_option_parses() {
    let args = parse(&["--html".to_string(), "out/report.html".to_string()]).unwrap();
    assert_eq!(args.html, Some(PathBuf::from("out/report.html")));
    assert_eq!(parse(&[]).unwrap().html, None);
}

#[test]
fn html_escapes_activity_names() {
    let run = run_on("1", "<script>alert('x')</script> & co", 3);
    let args = html_args(None);
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(!page.contains("<script>alert"));
    assert!(page.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; co"));
}

#[test]
fn html_explains_a_missing_hr_max() {
    let run = run_on("1", "Run", 3);
    let without = html_args(None);
    let page = render(&build(&[&run], &without), &without, august(3));
    assert!(page.contains("Needs --hr-max"));
    assert!(!page.contains("Time in HR zones"));

    let with = html_args(Some(186));
    let page = render(&build(&[&run], &with), &with, august(3));
    assert!(!page.contains("Needs --hr-max"));
    assert!(page.contains("Time in HR zones"));
    assert!(page.contains("TRIMP load"));
}

#[test]
fn html_keeps_empty_weeks_in_the_charts() {
    let (first, second) = (run_on("1", "Run", 3), run_on("2", "Run", 24));
    let args = html_args(None);
    let page = render(&build(&[&first, &second], &args), &args, august(24));
    // Monday 3 to Monday 24 August, with two weeks between them without a session.
    assert!(page.contains("2026-W33\nno sessions"));
    assert!(page.contains("2026-W34\nno sessions"));
}

#[test]
fn html_handles_sessions_without_hr_or_running() {
    let mut walk = activity(&track(&[(300, 50.0, 1.2)]));
    walk.name = "Walk".to_string();
    for point in &mut walk.points {
        point.hr = None;
    }
    let args = html_args(Some(186));
    let page = render(&build(&[&walk], &args), &args, august(7));
    assert!(page.contains("No data in this range."));
    assert!(page.contains("id=\"s-1\""));
}

#[test]
fn html_shows_an_empty_range_without_failing() {
    let args = html_args(None);
    let metrics = build(&[], &args);
    let page = render(&metrics, &args, august(7));
    assert!(page.contains("0 sessions"));
    assert!(page.contains("body data-range=\"all\""));
}

#[test]
fn html_defaults_to_all_when_nothing_is_recent() {
    let run = run_on("1", "Run", 3);
    let args = html_args(None);
    let later = Date {
        year: 2027,
        month: 8,
        day: 3,
    };
    let page = render(&build(&[&run], &args), &args, later);
    assert!(page.contains("body data-range=\"all\""));
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("body data-range=\"recent\""));
}

#[test]
fn html_holds_no_positions_or_network_addresses() {
    let mut run = run_on("1", "Run", 3);
    for point in &mut run.points {
        point.lat = 57.7412345;
        point.lon = 12.1298765;
    }
    let args = html_args(Some(186));
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(!page.contains("57.74"));
    assert!(!page.contains("12.12"));
    assert!(!page.contains("http"));
}

/// `run_on`, heading north-east from a starting position.
fn run_from(id: &str, day: u32, lat: f64, lon: f64) -> Activity {
    let mut run = run_on(id, "Run", day);
    for (i, point) in run.points.iter_mut().enumerate() {
        point.lat = lat + i as f64 * 0.00002;
        point.lon = lon + i as f64 * 0.00003;
    }
    run
}

#[test]
fn serve_options_parse() {
    let args = parse(&[
        "--serve".to_string(),
        "--port".to_string(),
        "9000".to_string(),
        "--map".to_string(),
    ])
    .unwrap();
    assert!(args.serve && args.map);
    assert_eq!(args.port, 9000);
    assert_eq!(Args::default().port, 8765);
    assert!(parse(&["--port".to_string(), "0".to_string()]).is_err());
    assert!(parse(&["--port".to_string(), "http".to_string()]).is_err());
}

#[test]
fn form_bodies_decode() {
    assert_eq!(
        decode_form("route=3&name=Sj%C3%B6+loop%23").unwrap(),
        [
            ("route".to_string(), "3".to_string()),
            ("name".to_string(), "Sjö loop#".to_string()),
        ]
    );
    assert_eq!(
        decode_form("name=").unwrap(),
        [("name".to_string(), String::new())]
    );
    assert!(decode_form("name=%zz").is_none());
    assert!(decode_form("name=%C3").is_none());
}

fn rename_request(body: &str) -> Request {
    Request {
        method: "POST".to_string(),
        path: "/route-name".to_string(),
        headers: vec![
            ("host".to_string(), "127.0.0.1:8765".to_string()),
            ("x-digirun".to_string(), "rename".to_string()),
        ],
        body: body.as_bytes().to_vec(),
    }
}

#[test]
fn served_report_renames_routes() {
    let runs = four_runs();
    let mut metrics = named_metrics(&runs);
    let path = temp_dir("serve").join("routes.txt");
    std::fs::write(&path, "# mine\n1 = Home\n").unwrap();
    let mut state = State::new(&mut metrics, &Args::default(), path.clone(), august(21));
    let get = Request {
        method: "GET".to_string(),
        path: "/".to_string(),
        ..Request::default()
    };
    let page = String::from_utf8(handle(&get, &mut state).body).unwrap();
    assert!(page.contains("<body data-range=\"recent\" data-editable>"));
    assert!(page.contains("<div class=\"label\">Home</div>"));

    assert_eq!(
        handle(&rename_request("route=0&name=Lake+loop"), &mut state).status,
        204
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# mine\n2 = Lake loop\n"
    );
    let page = String::from_utf8(handle(&get, &mut state).body).unwrap();
    assert!(page.contains("<div class=\"label\">Lake loop</div>"));

    // An empty name goes back to the automatic one.
    assert_eq!(
        handle(&rename_request("route=0&name="), &mut state).status,
        204
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
    let page = String::from_utf8(handle(&get, &mut state).body).unwrap();
    assert!(page.contains("<div class=\"label\">R1 · "));

    assert_eq!(
        handle(&rename_request("route=0&name=A%23B"), &mut state).status,
        400
    );
    assert_eq!(
        handle(&rename_request("route=9&name=X"), &mut state).status,
        400
    );
    let mut unmarked = rename_request("route=0&name=X");
    unmarked.headers.retain(|(n, _)| n != "x-digirun");
    assert_eq!(handle(&unmarked, &mut state).status, 403);
    let mut foreign = rename_request("route=0&name=X");
    foreign.headers[0].1 = "evil.example:8765".to_string();
    assert_eq!(handle(&foreign, &mut state).status, 403);
    let lost = Request {
        path: "/routes.txt".to_string(),
        ..get.clone()
    };
    assert_eq!(handle(&lost, &mut state).status, 404);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
}

#[test]
fn map_needs_html() {
    let map = "--map".to_string();
    assert!(parse(std::slice::from_ref(&map)).is_err());
    let html = ["--html".to_string(), "r.html".to_string(), map];
    assert!(parse(&html).unwrap().map);
    assert!(!parse(&[]).unwrap().map);
}

#[test]
fn html_adds_the_map_and_positions_only_when_asked() {
    let run = run_from("1", 3, 57.7412345, 12.1298765);
    let plain = html_args(None);
    let page = render(&build(&[&run], &plain), &plain, august(3));
    assert!(!page.contains("data-editable"));
    assert!(!page.contains("map-toggle"));
    assert!(!page.to_lowercase().contains("leaflet"));
    assert!(!page.contains("s2cloudless"));
    assert!(!page.contains("data-track"));

    let with_map = Args {
        map: true,
        ..html_args(None)
    };
    let page = render(&build(&[&run], &with_map), &with_map, august(3));
    assert!(page.contains("id=\"map-toggle\" aria-pressed=\"true\""));
    assert!(page.contains("s2cloudless-2025_3857"));
    assert!(page.contains("EOX IT Services GmbH"));
    assert!(page.contains("[57.74123,12.12988]"));
    assert!(page.contains("keep it private"));
    // The embedded scripts must not end their own script element early.
    assert_eq!(
        page.matches("<script>").count(),
        page.matches("</script>").count()
    );
}

#[test]
fn map_tolerates_a_session_without_a_track() {
    let run = run_on("1", "Run", 3);
    let args = Args {
        map: true,
        ..html_args(None)
    };
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("No GPS track for this session."));
    assert!(!page.contains("data-track"));
}

#[test]
fn route_frame_uses_one_scale_for_both_axes() {
    let square = [(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];
    let frame = Frame::fit(&square, (320.0, 220.0), 20.0).unwrap();
    let (left, bottom) = frame.px((0.0, 0.0));
    let (right, top) = frame.px((1000.0, 1000.0));
    assert!((right - left - (bottom - top)).abs() < 1e-9);
    assert!(top < bottom, "north is up");
    assert!(top >= 20.0 && bottom <= 200.0);
    assert_eq!(Frame::fit(&[], (320.0, 220.0), 20.0), None);
}

#[test]
fn html_draws_routes_without_their_positions() {
    let run = run_from("1", 3, 57.7412345, 12.1298765);
    let args = html_args(None);
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("class=\"card route-card\" href=\"#sessions\" data-route=\"0\""));
    assert!(page.contains("class=\"track run\""));
    assert!(!page.contains("57.7"));
    assert!(!page.contains("12.1298"));
}

#[test]
fn html_notes_a_session_without_a_track() {
    let run = run_on("1", "Run", 3);
    let args = html_args(None);
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("No GPS track for this session."));
    assert!(page.contains("No GPS tracks in this range."));
}

#[test]
fn html_overview_leaves_out_a_faraway_route() {
    let (a, b, far) = (
        run_from("1", 3, 57.74, 12.12),
        run_from("2", 4, 57.75, 12.13),
        run_from("3", 5, 40.0, -3.7),
    );
    let args = html_args(None);
    let page = render(&build(&[&a, &b, &far], &args), &args, august(5));
    assert!(page.contains("1 further away than 15 km left out here"));
    // The faraway route keeps its card.
    assert_eq!(page.matches("data-route=\"2\" title=").count(), 2);
}
