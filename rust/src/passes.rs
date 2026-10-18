//! Passes: the continuous run and walk stretches of an activity, split on cadence.
//!
//! Garmin's laps are workout steps or 1 km auto-splits, so one lap can hold both
//! running and walking. Passes ignore them: each trackpoint interval is labelled
//! from a rolling median of cadence with a hysteresis band, runs of the same label
//! become passes, and passes shorter than `--min-pass` join a neighbour.

use crate::analyze::{mean, round, stdev, RUN, WALK};
use crate::args::{Args, Method, AMBIGUOUS_CADENCE};
use crate::model::{haversine, Activity, Point};

/// What the labels of a pass came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cadence,
    /// The activity has no cadence, so a rolling pace decided.
    Pace,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Cadence => "cadence",
            Source::Pace => "pace",
        }
    }
}

/// One pass: the intervals ending at points `start + 1 ..= end`.
///
/// Consecutive passes share their boundary point, so every interval belongs to
/// exactly one pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    pub class: &'static str,
    pub start: usize,
    pub end: usize,
    /// 1-based TCX lap numbers the pass overlaps.
    pub laps: Vec<usize>,
    pub source: Source,
    pub stats: PassStats,
    /// Outside --max-pace/--min-pace, so left out of the totals.
    pub excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PassStats {
    pub start_offset_s: f64,
    pub time_s: f64,
    pub distance_m: f64,
    pub pace: Option<f64>,
    /// Over the `--window` sub-windows of the pass; needs two of them.
    pub pace_sd: Option<f64>,
    pub pace_min: Option<f64>,
    pub pace_max: Option<f64>,
    pub cad_mean: Option<f64>,
    pub cad_sd: Option<f64>,
    pub cad_min: Option<f64>,
    pub cad_max: Option<f64>,
    pub hr_mean: Option<f64>,
    pub hr_min: Option<f64>,
    pub hr_max: Option<f64>,
    /// Mean HR of the last third minus the first third.
    pub hr_drift: Option<f64>,
    pub elev_gain: Option<f64>,
    pub elev_loss: Option<f64>,
    /// Metres per stride.
    pub stride_m: Option<f64>,
    /// Seconds of sub-windows outside --max-pace/--min-pace, left out of the stats.
    pub stopped_s: f64,
}

/// An interval between point `k - 1` and point `k`.
struct Step {
    dt: f64,
    dm: f64,
    /// A pause longer than --gap: not counted, and it ends the pass.
    gap: bool,
}

fn steps(points: &[Point], args: &Args) -> Vec<Step> {
    let mut out = vec![Step {
        dt: 0.0,
        dm: 0.0,
        gap: false,
    }];
    for k in 1..points.len() {
        let (prev, point) = (&points[k - 1], &points[k]);
        let dt = point.time.seconds_since(&prev.time);
        let mut dm = point.distance_m - prev.distance_m;
        if dm < 0.0 {
            // TCX DistanceMeters resets at a lap boundary: bridge it with the positions.
            dm = if prev.lat != 0.0 && point.lat != 0.0 {
                haversine(prev.lat, prev.lon, point.lat, point.lon)
            } else {
                0.0
            };
        }
        let gap = dt > args.gap as f64;
        out.push(Step {
            dt: if gap || dt < 0.0 { 0.0 } else { dt },
            dm: if gap { 0.0 } else { dm },
            gap,
        });
    }
    out
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// Seconds since the first point, for each point.
fn offsets(points: &[Point]) -> Vec<f64> {
    points
        .iter()
        .map(|p| p.time.seconds_since(&points[0].time))
        .collect()
}

/// Rolling median of cadence over ±`half` seconds; None where no point has cadence.
fn smoothed_cadence(points: &[Point], t: &[f64], half: f64) -> Vec<Option<f64>> {
    let mut lo = 0;
    let mut hi = 0;
    let mut out = Vec::with_capacity(points.len());
    for k in 0..points.len() {
        while t[lo] < t[k] - half {
            lo += 1;
        }
        while hi + 1 < points.len() && t[hi + 1] <= t[k] + half {
            hi += 1;
        }
        let mut values: Vec<f64> = points[lo..=hi].iter().filter_map(|p| p.cadence).collect();
        out.push(if values.is_empty() {
            None
        } else {
            Some(median(&mut values))
        });
    }
    out
}

/// Rolling pace (s/km) over ±`half` seconds of counted steps; None when no distance.
fn rolling_pace(steps: &[Step], t: &[f64], half: f64) -> Vec<Option<f64>> {
    // Prefix sums of time and distance, so each window is two lookups.
    let mut time = vec![0.0; steps.len() + 1];
    let mut dist = vec![0.0; steps.len() + 1];
    for (j, step) in steps.iter().enumerate() {
        time[j + 1] = time[j] + step.dt;
        dist[j + 1] = dist[j] + step.dm;
    }
    let (mut lo, mut hi) = (0, 0);
    (0..steps.len())
        .map(|k| {
            while t[lo] < t[k] - half {
                lo += 1;
            }
            while hi + 1 < steps.len() && t[hi + 1] <= t[k] + half {
                hi += 1;
            }
            let d = dist[hi + 1] - dist[lo];
            (d > 0.0).then(|| (time[hi + 1] - time[lo]) / (d / 1000.0))
        })
        .collect()
}

/// The label of each step (index 0 is unused).
fn labels(points: &[Point], steps: &[Step], args: &Args) -> (Vec<&'static str>, Source) {
    let t = offsets(points);
    let cadence = smoothed_cadence(points, &t, args.smooth as f64 / 2.0);
    let has_cadence = points.iter().any(|p| p.cadence.is_some_and(|c| c > 0.0));
    let source = if has_cadence && args.method != Method::Pace {
        Source::Cadence
    } else {
        Source::Pace
    };
    let pace = rolling_pace(steps, &t, args.window as f64 / 2.0);
    let by_pace = |k: usize| match pace[k] {
        Some(pace) if pace <= args.pace_threshold as f64 => RUN,
        _ => WALK,
    };

    let mut out = vec![WALK; steps.len()];
    let mut state: Option<&'static str> = None;
    for k in 1..steps.len() {
        let label = match (source, cadence[k]) {
            (Source::Pace, _) => by_pace(k),
            (Source::Cadence, Some(c)) if args.method == Method::Cadence => {
                if c >= args.cadence_threshold as f64 {
                    RUN
                } else {
                    WALK
                }
            }
            (Source::Cadence, Some(c)) if c > AMBIGUOUS_CADENCE.1 as f64 => RUN,
            (Source::Cadence, Some(c)) if c < AMBIGUOUS_CADENCE.0 as f64 => WALK,
            // Inside the band, or no cadence here: hold the state, else ask the pace.
            (Source::Cadence, _) => state.unwrap_or_else(|| by_pace(k)),
        };
        state = Some(label);
        out[k] = label;
    }
    (out, source)
}

/// A pass before its stats: steps `first ..= last`, and whether a gap precedes it.
#[derive(Debug, Clone)]
struct Raw {
    class: &'static str,
    first: usize,
    last: usize,
    time: f64,
    after_gap: bool,
}

fn coalesce(raws: Vec<Raw>) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    for raw in raws {
        match out.last_mut() {
            Some(prev) if prev.class == raw.class && !raw.after_gap => {
                prev.last = raw.last;
                prev.time += raw.time;
            }
            _ => out.push(raw),
        }
    }
    out
}

/// Merge passes under `min_pass` seconds into a neighbour, shortest first.
fn absorb(mut raws: Vec<Raw>, min_pass: f64) -> Vec<Raw> {
    loop {
        raws = coalesce(raws);
        // Neighbours across a gap are not joined; a pass alone between gaps stays.
        let joinable = |i: usize| {
            (i > 0 && !raws[i].after_gap) || (i + 1 < raws.len() && !raws[i + 1].after_gap)
        };
        let shortest = (0..raws.len())
            .filter(|&i| raws[i].time < min_pass && joinable(i))
            .min_by(|&a, &b| raws[a].time.total_cmp(&raws[b].time));
        let Some(i) = shortest else {
            return raws;
        };
        let left = (i > 0 && !raws[i].after_gap).then(|| i - 1);
        let right = (i + 1 < raws.len() && !raws[i + 1].after_gap).then_some(i + 1);
        let target = match (left, right) {
            (Some(l), Some(r)) => {
                if raws[l].time >= raws[r].time {
                    l
                } else {
                    r
                }
            }
            (Some(l), None) => l,
            (None, Some(r)) => r,
            (None, None) => unreachable!("filtered by joinable"),
        };
        raws[i].class = raws[target].class;
    }
}

/// Split an activity into passes and compute their stats.
pub fn build_passes(activity: &Activity, args: &Args) -> Vec<Pass> {
    let points = &activity.points;
    if points.len() < 2 {
        return Vec::new();
    }
    let steps = steps(points, args);
    let (labels, source) = labels(points, &steps, args);

    let mut raws: Vec<Raw> = Vec::new();
    let mut after_gap = false;
    for k in 1..steps.len() {
        if steps[k].gap {
            after_gap = true;
            continue;
        }
        if steps[k].dt == 0.0 {
            continue;
        }
        match raws.last_mut() {
            Some(prev) if prev.class == labels[k] && !after_gap => {
                prev.last = k;
                prev.time += steps[k].dt;
            }
            _ => raws.push(Raw {
                class: labels[k],
                first: k,
                last: k,
                time: steps[k].dt,
                after_gap,
            }),
        }
        after_gap = false;
    }
    let raws = absorb(raws, args.min_pass as f64);

    let t = offsets(points);
    raws.iter()
        .map(|raw| {
            let start = raw.first - 1;
            let end = raw.last;
            let stats = pass_stats(points, &steps, &t, start, end, args);
            // Whole passes outside the guard, or with nothing left after it.
            let excluded = stats.pace.is_none_or(|pace| {
                args.max_pace.is_some_and(|max| pace > max as f64)
                    || args.min_pace.is_some_and(|min| pace < min as f64)
            });
            Pass {
                class: raw.class,
                start,
                end,
                laps: activity
                    .laps
                    .iter()
                    .enumerate()
                    .filter(|(_, lap)| lap.start_index <= end && lap.end_index > start)
                    .map(|(i, _)| i + 1)
                    .collect(),
                source,
                stats,
                excluded,
            }
        })
        .collect()
}

fn opt_round(value: Option<f64>, digits: usize) -> Option<f64> {
    value.map(|v| round(v, digits))
}

fn spread(values: &[f64]) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    if values.is_empty() {
        return (None, None, None, None);
    }
    (
        Some(mean(values)),
        (values.len() > 1).then(|| stdev(values)),
        Some(values.iter().copied().fold(f64::INFINITY, f64::min)),
        Some(values.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
    )
}

fn pass_stats(
    points: &[Point],
    steps: &[Step],
    t: &[f64],
    start: usize,
    end: usize,
    args: &Args,
) -> PassStats {
    // Cut the pass into --window sub-windows, as the segment report does. A
    // sub-window outside --max-pace/--min-pace (standing about, GPS glitches) is
    // left out of every stat below, the way the report drops such segments.
    let guarded = |time: f64, distance: f64| {
        let pace = time / (distance / 1000.0);
        args.max_pace.is_some_and(|max| pace > max as f64)
            || args.min_pace.is_some_and(|min| pace < min as f64)
    };
    let mut counted: Vec<usize> = Vec::new();
    let mut window_paces: Vec<f64> = Vec::new();
    let mut stopped = 0.0;
    let mut chunk: Vec<usize> = Vec::new();
    let (mut wt, mut wd) = (0.0, 0.0);
    let steps_in_pass: Vec<usize> = (start + 1..=end).filter(|&k| steps[k].dt > 0.0).collect();
    for (n, &k) in steps_in_pass.iter().enumerate() {
        chunk.push(k);
        wt += steps[k].dt;
        wd += steps[k].dm;
        let full = wt >= args.window as f64;
        if !full && n + 1 < steps_in_pass.len() {
            continue;
        }
        // The tail of the pass is judged too, once it is long enough to measure.
        if wt >= args.min_segment as f64 && guarded(wt, wd) {
            stopped += wt;
        } else {
            if full && wd >= args.min_distance {
                window_paces.push(wt / (wd / 1000.0));
            }
            counted.append(&mut chunk);
        }
        chunk.clear();
        wt = 0.0;
        wd = 0.0;
    }
    let time: f64 = counted.iter().map(|&k| steps[k].dt).sum();
    let distance: f64 = counted.iter().map(|&k| steps[k].dm).sum();
    let (_, pace_sd, pace_min, pace_max) = if window_paces.len() > 1 {
        spread(&window_paces)
    } else {
        (None, None, None, None)
    };

    let cadences: Vec<f64> = counted
        .iter()
        .filter_map(|&k| points[k].cadence)
        .filter(|&c| c > 0.0)
        .collect();
    let (cad_mean, cad_sd, cad_min, cad_max) = spread(&cadences);

    let hrs: Vec<(f64, f64)> = counted
        .iter()
        .filter_map(|&k| points[k].hr.map(|hr| (t[k], hr)))
        .collect();
    let hr_values: Vec<f64> = hrs.iter().map(|x| x.1).collect();
    let (hr_mean, _, hr_min, hr_max) = spread(&hr_values);
    let (t0, t1) = (t[start], t[end]);
    let third = (t1 - t0) / 3.0;
    let first: Vec<f64> = hrs
        .iter()
        .filter(|x| x.0 <= t0 + third)
        .map(|x| x.1)
        .collect();
    let last: Vec<f64> = hrs
        .iter()
        .filter(|x| x.0 >= t1 - third)
        .map(|x| x.1)
        .collect();
    let hr_drift = (!first.is_empty() && !last.is_empty()).then(|| mean(&last) - mean(&first));

    let (mut gain, mut loss, mut has_alt) = (0.0, 0.0, false);
    for &k in &counted {
        if let (Some(a), Some(b)) = (points[k - 1].altitude, points[k].altitude) {
            has_alt = true;
            let d = b - a;
            if d > 0.0 {
                gain += d;
            } else {
                loss -= d;
            }
        }
    }

    let pace = (distance > 0.0).then(|| time / (distance / 1000.0));
    let stride_m = cad_mean
        .filter(|_| time > 0.0)
        .map(|c| distance / (c * time / 60.0));

    PassStats {
        start_offset_s: t[start],
        stopped_s: round(stopped, 1),
        time_s: round(time, 1),
        distance_m: round(distance, 1),
        pace: opt_round(pace, 1),
        pace_sd: opt_round(pace_sd, 1),
        pace_min: opt_round(pace_min, 1),
        pace_max: opt_round(pace_max, 1),
        cad_mean: opt_round(cad_mean, 1),
        cad_sd: opt_round(cad_sd, 1),
        cad_min: opt_round(cad_min, 1),
        cad_max: opt_round(cad_max, 1),
        hr_mean: opt_round(hr_mean, 1),
        hr_min: opt_round(hr_min, 1),
        hr_max: opt_round(hr_max, 1),
        hr_drift: opt_round(hr_drift, 1),
        elev_gain: has_alt.then(|| round(gain, 1)),
        elev_loss: has_alt.then(|| round(loss, 1)),
        stride_m: opt_round(stride_m, 2),
    }
}

/// Totals for a group of passes of one class: weighted by time where it matters.
#[derive(Debug, Clone, PartialEq)]
pub struct Totals {
    pub passes: usize,
    pub time_s: f64,
    pub distance_m: f64,
    pub pace: Option<f64>,
    /// Spread of the pass paces.
    pub pace_sd: Option<f64>,
    pub mean_pass_s: f64,
    pub median_pass_s: f64,
    pub cad_mean: Option<f64>,
    pub hr_mean: Option<f64>,
}

pub fn totals(passes: &[&Pass]) -> Option<Totals> {
    if passes.is_empty() {
        return None;
    }
    let time: f64 = passes.iter().map(|p| p.stats.time_s).sum();
    let distance: f64 = passes.iter().map(|p| p.stats.distance_m).sum();
    let weighted = |value: fn(&PassStats) -> Option<f64>| {
        let pairs: Vec<(f64, f64)> = passes
            .iter()
            .filter_map(|p| value(&p.stats).map(|v| (v, p.stats.time_s)))
            .collect();
        let weight: f64 = pairs.iter().map(|x| x.1).sum();
        (weight > 0.0).then(|| pairs.iter().map(|x| x.0 * x.1).sum::<f64>() / weight)
    };
    let paces: Vec<f64> = passes.iter().filter_map(|p| p.stats.pace).collect();
    let mut lengths: Vec<f64> = passes.iter().map(|p| p.stats.time_s).collect();
    Some(Totals {
        passes: passes.len(),
        time_s: round(time, 1),
        distance_m: round(distance, 1),
        pace: (distance > 0.0).then(|| round(time / (distance / 1000.0), 1)),
        pace_sd: (paces.len() > 1).then(|| round(stdev(&paces), 1)),
        mean_pass_s: round(time / passes.len() as f64, 1),
        median_pass_s: round(median(&mut lengths), 1),
        cad_mean: opt_round(weighted(|s| s.cad_mean), 1),
        hr_mean: opt_round(weighted(|s| s.hr_mean), 1),
    })
}
