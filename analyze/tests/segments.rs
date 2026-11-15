//! Walk/run segments, their stats, filters and CSVs.

mod common;

use std::collections::BTreeSet;

use garmin_analyze::analyze::{
    build_segments, classify, filter_activities, filter_segments, in_range, stats,
};
use garmin_analyze::args::{Args, Method};
use garmin_analyze::csvout::write_csvs;
use garmin_analyze::dt::Date;
use garmin_analyze::model::{Activity, Segment};
use garmin_analyze::num::round;

use common::*;

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
    let pace_only = Args {
        method: Method::Pace,
        ..Args::default()
    };
    assert_eq!(classify(539.0, Some(50.0), &pace_only), "run");
    assert_eq!(classify(541.0, Some(90.0), &pace_only), "walk");

    let cadence_only = Args {
        method: Method::Cadence,
        ..Args::default()
    };
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
