//! Classification, segmentation and per-class stats.

use std::collections::BTreeSet;

use crate::args::{Args, Method};
use crate::model::{Activity, Point, Segment, Stats};

pub const RUN: &str = "run";
pub const WALK: &str = "walk";

/// Python's `round(value, digits)`: the correctly rounded decimal, read back as
/// a float, so the CSVs hold short, stable numbers.
pub fn round(value: f64, digits: usize) -> f64 {
    format!("{:.*}", digits, value)
        .parse::<f64>()
        .unwrap_or(value)
}

pub(crate) fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// Sample standard deviation (`statistics.stdev`), which needs at least two values.
pub(crate) fn stdev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    let sum: f64 = values.iter().map(|v| (v - m) * (v - m)).sum();
    (sum / (values.len() - 1) as f64).sqrt()
}

/// Return "run" or "walk" for one segment.
pub fn classify(pace: f64, cadence: Option<f64>, args: &Args) -> &'static str {
    let by_pace = |pace: f64| {
        if pace <= args.pace_threshold as f64 {
            RUN
        } else {
            WALK
        }
    };

    match args.method {
        Method::Pace | Method::Lap => by_pace(pace),
        Method::Cadence => match cadence {
            Some(cadence) => {
                if cadence >= args.cadence_threshold as f64 {
                    RUN
                } else {
                    WALK
                }
            }
            None => by_pace(pace),
        },
        Method::Hybrid => {
            if let Some(cadence) = cadence {
                if cadence < crate::args::AMBIGUOUS_CADENCE.0 as f64 {
                    return WALK;
                }
                if cadence > crate::args::AMBIGUOUS_CADENCE.1 as f64 {
                    return RUN;
                }
            }
            by_pace(pace)
        }
    }
}

/// Split an activity into non-overlapping windows of `window` seconds.
pub fn build_segments(activity: &Activity, args: &Args) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    // Bucket of (seconds, metres, cadence) since the last emitted segment.
    let mut bucket: Vec<(f64, f64, Option<f64>)> = Vec::new();
    let mut total_time = 0.0;
    let mut total_distance = 0.0;
    let mut prev: Option<&Point> = None;

    for point in &activity.points {
        if let Some(prev) = prev {
            let dt = point.time.seconds_since(&prev.time);
            let dm = point.distance_m - prev.distance_m;
            if dt > args.gap as f64 {
                bucket.clear(); // stopped time: do not count it as walking
                total_time = 0.0;
                total_distance = 0.0;
            } else if dm < 0.0 {
                bucket.clear(); // TCX DistanceMeters resets at a lap boundary
                total_time = 0.0;
                total_distance = 0.0;
            } else if dt > 0.0 && dm > 0.0 {
                bucket.push((dt, dm, point.cadence));
                total_time += dt;
                total_distance += dm;
            }
        }
        prev = Some(point);

        if total_time < args.window as f64 {
            continue;
        }
        if total_time >= args.min_segment as f64 && total_distance >= args.min_distance {
            let cadences: Vec<f64> = bucket.iter().filter_map(|x| x.2).collect();
            let pace = total_time / (total_distance / 1000.0);
            let cadence = if cadences.is_empty() {
                None
            } else {
                Some(mean(&cadences))
            };
            segments.push(Segment {
                activity_id: activity.activity_id.clone(),
                date: activity.date,
                time: point.time,
                duration_s: total_time,
                distance_m: total_distance,
                pace_s_per_km: pace,
                cadence,
                class: classify(pace, cadence, args),
            });
            bucket.clear();
            total_time = 0.0;
            total_distance = 0.0;
        }
    }

    segments
}

/// Treat each TCX lap as one segment, labelled by its own pace.
pub fn lap_segments(activity: &Activity, args: &Args) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for lap in &activity.laps {
        if lap.duration_s <= 0.0 || lap.distance_m <= 0.0 {
            continue;
        }
        let pace = lap.duration_s / (lap.distance_m / 1000.0);
        let cadences: Vec<f64> = activity.points[lap.start_index..=lap.end_index]
            .iter()
            .filter_map(|p| p.cadence)
            .collect();
        let cadence = if cadences.is_empty() {
            None
        } else {
            Some(mean(&cadences))
        };
        segments.push(Segment {
            activity_id: activity.activity_id.clone(),
            date: activity.date,
            time: activity.points[lap.start_index].time,
            duration_s: lap.duration_s,
            distance_m: lap.distance_m,
            pace_s_per_km: pace,
            cadence,
            class: classify(pace, cadence, args),
        });
    }
    segments
}

/// Pace (total time / total distance) and the stdev of the segment paces, plus
/// cadence where the segments have it.
///
/// Averaging the segment paces would read slower: a slow segment covers less
/// ground in the same time, so it would weigh in more.
pub fn stats(groups: &[&Segment]) -> Stats {
    let paces: Vec<f64> = groups.iter().map(|s| s.pace_s_per_km).collect();
    let cadences: Vec<f64> = groups.iter().filter_map(|s| s.cadence).collect();
    let time: f64 = groups.iter().map(|s| s.duration_s).sum();
    let distance: f64 = groups.iter().map(|s| s.distance_m).sum();

    Stats {
        segments: paces.len(),
        time_s: round(time, 1),
        distance_km: round(distance / 1000.0, 2),
        mean: if distance > 0.0 {
            round(time / (distance / 1000.0), 1)
        } else {
            0.0
        },
        stdev: if paces.len() > 1 {
            round(stdev(&paces), 1)
        } else {
            0.0
        },
        min: if paces.is_empty() {
            0.0
        } else {
            round(paces.iter().copied().fold(f64::INFINITY, f64::min), 1)
        },
        max: if paces.is_empty() {
            0.0
        } else {
            round(paces.iter().copied().fold(f64::NEG_INFINITY, f64::max), 1)
        },
        cad_mean: if cadences.is_empty() {
            None
        } else {
            Some(round(mean(&cadences), 1))
        },
        cad_stdev: if cadences.len() > 1 {
            Some(round(stdev(&cadences), 1))
        } else {
            None
        },
    }
}

/// Keep activities whose type is in `wanted`; returns (kept, skipped, unknown).
///
/// Activities whose type is unknown (not in summary.csv) are skipped, since the
/// point of the filter is to keep other sports out of the stats.
pub fn filter_activities<'a>(
    activities: &'a [&'a Activity],
    wanted: &Option<BTreeSet<String>>,
) -> (Vec<&'a Activity>, usize, usize) {
    match wanted {
        None => (activities.to_vec(), 0, 0),
        Some(wanted) => {
            let mut kept: Vec<&Activity> = Vec::new();
            let mut skipped = 0usize;
            let mut unknown = 0usize;
            for activity in activities {
                if activity.r#type.is_empty() {
                    unknown += 1;
                } else if wanted.contains(&activity.r#type) {
                    kept.push(activity);
                } else {
                    skipped += 1;
                }
            }
            (kept, skipped, unknown)
        }
    }
}

/// Drop segments outside the pace window; returns (kept, dropped_slow, dropped_fast).
pub fn filter_segments<'a>(
    segments: &'a [&'a Segment],
    args: &Args,
) -> (Vec<&'a Segment>, usize, usize) {
    let max_pace = args.max_pace;
    let min_pace = args.min_pace;
    if max_pace.is_none() && min_pace.is_none() {
        return (segments.to_vec(), 0, 0);
    }

    let mut kept: Vec<&Segment> = Vec::new();
    let mut slow = 0usize;
    let mut fast = 0usize;
    for s in segments {
        if let Some(max) = max_pace {
            if s.pace_s_per_km > max as f64 {
                slow += 1;
                continue;
            }
        }
        if let Some(min) = min_pace {
            if s.pace_s_per_km < min as f64 {
                fast += 1;
                continue;
            }
        }
        kept.push(s);
    }
    (kept, slow, fast)
}

pub fn in_range(activity: &Activity, args: &Args) -> bool {
    if let Some(start) = args.start {
        if activity.date < start {
            return false;
        }
    }
    if let Some(end) = args.end {
        if activity.date > end {
            return false;
        }
    }
    true
}
