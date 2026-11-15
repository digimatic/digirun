//! Cadence-split passes and their stats.

mod common;

use garmin_analyze::args::Args;
use garmin_analyze::passes::{Source, build_passes, climbs};
use garmin_analyze::segments::build_segments;

use common::*;

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
