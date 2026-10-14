//! Tests mirroring `tests/test_analyze.py`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use garmin_analyze::analyze::{
    build_segments, classify, filter_activities, filter_segments, in_range, round, stats,
};
use garmin_analyze::args::{Args, Method};
use garmin_analyze::csvout::write_csvs;
use garmin_analyze::dt::{Date, DateTime};
use garmin_analyze::load::{activity_types, load_activities};
use garmin_analyze::model::{Activity, Point, Segment};
use garmin_analyze::parse::read_gpx;
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
