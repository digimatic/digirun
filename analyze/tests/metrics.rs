//! Session and week metrics: zones, load, fitness, efforts.

mod common;

use garmin_analyze::args::Args;
use garmin_analyze::metrics::{
    acute_chronic, build, hr_zone, minetti_cost, prediction_source, previous_weeks, riegel, trimp,
    vdot,
};
use garmin_analyze::model::Activity;

use common::*;

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
