//! Tests mirroring `tests/test_analyze.py`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use garmin_analyze::analyze::{
    build_segments, classify, filter_activities, filter_segments, in_range, round, stats,
};
use garmin_analyze::args::{help, parse, Args, Method};
use garmin_analyze::csvout::write_csvs;
use garmin_analyze::detail_args;
use garmin_analyze::dt::{Date, DateTime};
use garmin_analyze::load::{activity_types, load_activities};
use garmin_analyze::model::{Activity, Lap, Point, Segment};
use garmin_analyze::parse::{read_gpx, read_tcx};
use garmin_analyze::passes::{build_passes, Source};
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
    assert_eq!(s.hr_drift, Some(19.9)); // last 40 s (145.0) minus first 40 s (125.125)
    assert_eq!((s.elev_gain, s.elev_loss), (Some(60.0), Some(60.0)));
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
fn detail_defaults_to_the_last_week() {
    let today = Date {
        year: 2026,
        month: 1,
        day: 3,
    };
    let args = detail_args(&Args::default(), today);
    assert_eq!(
        args.start,
        Some(Date {
            year: 2025,
            month: 12,
            day: 28,
        })
    );
    assert_eq!(args.end, None);

    // A range or an activity id keeps the dates as given.
    let given = Args {
        activities: Some(BTreeSet::from(["1".to_string()])),
        ..Args::default()
    };
    assert_eq!(detail_args(&given, today).start, None);
}

#[test]
fn detail_options_parse() {
    let argv: Vec<String> = [
        "--detail",
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

    assert!(args.detail);
    assert_eq!(
        args.activities,
        Some(BTreeSet::from(["1".to_string(), "2".to_string()]))
    );
    assert_eq!((args.min_pass, args.smooth), (30, 6));
    assert!(help().contains("--detail"));
}
