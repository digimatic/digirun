//! Running metrics per session and per ISO week, built on the passes.
//!
//! Everything is calculated over all the activities up to `--end`, so rolling
//! baselines can look back past the start of the displayed range. The report
//! narrows the rows to the range when it prints them.

use std::collections::BTreeSet;

use crate::args::Args;
use crate::dt::Date;
use crate::model::Activity;
use crate::num::{median, upper_median};
use crate::passes::{Pass, Step, build_passes, offsets, smoothed_altitude, steps, totals};
use crate::routes::{NO_ROUTE, assign_routes, describe_route, route_cells};
use crate::segments::{RUN, WALK, in_range};

pub const ZONES: usize = 5;
pub const EFFORT_DISTANCES_M: [f64; 3] = [1000.0, 3000.0, 5000.0];

/// A run pass must last this long before decoupling means anything, and its
/// first minutes are skipped while HR catches up with the effort.
const DECOUPLING_MIN_PASS_S: f64 = 1200.0;
const DECOUPLING_SKIP_S: f64 = 300.0;
/// The run pass before a walk, and how far into the walk HR is read.
const RECOVERY_MIN_RUN_S: f64 = 120.0;
const RECOVERY_WINDOW_S: f64 = 60.0;
/// Less time than this in zone 3 does not give a pace worth comparing.
const ZONE_PACE_MIN_S: f64 = 120.0;
const GRADE_LIMIT: f64 = 0.30;
const PREDICTION_MIN_DISTANCE_M: f64 = 3000.0;
const PREDICTION_MAX_AGE_DAYS: i64 = 90;
/// An effort this share running counts as a run rather than a walk/run.
pub const RUNNING_EFFORT_SHARE: f64 = 0.9;

/// The best time over one distance within a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Effort {
    pub distance_m: f64,
    pub time_s: f64,
    pub run_share: f64,
    pub date: Date,
    pub activity_id: String,
}

impl Effort {
    pub fn pace(&self) -> f64 {
        self.time_s / (self.distance_m / 1000.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session<'a> {
    pub activity: &'a Activity,
    /// Index of the route group, or None without GPS positions.
    pub route: Option<usize>,
    /// The route's name from routes.txt, or a description of its first track.
    pub route_name: String,
    pub run_s: f64,
    pub walk_s: f64,
    pub run_m: f64,
    pub walk_m: f64,
    pub longest_run_s: f64,
    /// The distance of that same run, which need not be the farthest.
    pub longest_run_m: f64,
    pub run_hr: Option<f64>,
    pub elev_gain: Option<f64>,
    /// Run speed in m/min per heartbeat/min.
    pub efficiency: Option<f64>,
    /// Efficiency lost from the first to the second half of the longest run, in %.
    pub decoupling: Option<f64>,
    pub zone3_pace: Option<f64>,
    /// Median HR drop over the first minute of the walks after a run.
    pub recovery: Option<f64>,
    pub grade_adjusted_pace: Option<f64>,
    /// Seconds per zone for all the time recorded; empty without --hr-max.
    pub zones_all: [f64; ZONES],
    /// Seconds per zone within the run passes.
    pub zones_run: [f64; ZONES],
    pub trimp: Option<f64>,
    /// Per `EFFORT_DISTANCES_M`: the best regardless of walking.
    pub efforts_any: Vec<Option<Effort>>,
    /// Per `EFFORT_DISTANCES_M`: the best that is almost entirely running.
    pub efforts_running: Vec<Option<Effort>>,
}

impl Session<'_> {
    pub fn km(&self) -> f64 {
        (self.run_m + self.walk_m) / 1000.0
    }

    pub fn run_pace(&self) -> Option<f64> {
        (self.run_m > 0.0).then(|| self.run_s / (self.run_m / 1000.0))
    }
}

/// One ISO week. Weeks without a session are kept, because a gap is information.
#[derive(Debug, Clone, PartialEq)]
pub struct Week {
    pub monday: Date,
    pub sessions: usize,
    pub longest_run_s: f64,
    pub longest_run_m: f64,
    pub run_s: f64,
    pub walk_s: f64,
    pub km: f64,
    /// The mean of the previous four weeks; None with less history than that.
    pub km_baseline: Option<f64>,
    pub zones_all: [f64; ZONES],
    pub zones_run: [f64; ZONES],
    pub trimp: Option<f64>,
}

impl Week {
    fn empty(monday: Date) -> Week {
        Week {
            monday,
            sessions: 0,
            longest_run_s: 0.0,
            longest_run_m: 0.0,
            run_s: 0.0,
            walk_s: 0.0,
            km: 0.0,
            km_baseline: None,
            zones_all: [0.0; ZONES],
            zones_run: [0.0; ZONES],
            trimp: None,
        }
    }

    pub fn label(&self) -> String {
        let (year, week) = self.monday.isocalendar();
        format!("{year}-W{week:02}")
    }

    pub fn run_share(&self) -> Option<f64> {
        let total = self.run_s + self.walk_s;
        (total > 0.0).then(|| self.run_s / total * 100.0)
    }

    /// Whether the week overlaps --start/--end.
    pub fn in_range(&self, args: &Args) -> bool {
        args.start.is_none_or(|s| self.monday.plus_days(6) >= s)
            && args.end.is_none_or(|e| self.monday <= e)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metrics<'a> {
    pub sessions: Vec<Session<'a>>,
    pub weeks: Vec<Week>,
}

/// The HR zone (0 = 50-60% of max .. 4 = 90%+), or None below 50%.
pub fn hr_zone(hr: f64, hr_max: f64) -> Option<usize> {
    // Tens of a percent are compared after multiplying, so 120 of 200 is exactly 60%.
    let band = (hr * 10.0 / hr_max).floor() as i64;
    (band >= 5).then(|| ((band - 5) as usize).min(ZONES - 1))
}

/// Edwards TRIMP: minutes in each zone times the zone number.
pub fn trimp(zones: &[f64; ZONES]) -> f64 {
    zones
        .iter()
        .enumerate()
        .map(|(i, seconds)| seconds / 60.0 * (i + 1) as f64)
        .sum()
}

/// Minetti's cost of running in J/kg/m at a grade given as a fraction.
pub fn minetti_cost(grade: f64) -> f64 {
    let i = grade.clamp(-GRADE_LIMIT, GRADE_LIMIT);
    155.4 * i.powi(5) - 30.4 * i.powi(4) - 43.3 * i.powi(3) + 46.3 * i.powi(2) + 19.5 * i + 3.6
}

/// Riegel's prediction of the time over `to_m` from a time over `from_m`.
pub fn riegel(time_s: f64, from_m: f64, to_m: f64) -> f64 {
    time_s * (to_m / from_m).powf(1.06)
}

/// Daniels' VDOT for a performance.
#[allow(clippy::unreadable_literal)] // the published coefficients, as written
pub fn vdot(distance_m: f64, time_s: f64) -> f64 {
    let minutes = time_s / 60.0;
    let velocity = distance_m / minutes;
    let oxygen = -4.6 + 0.182258 * velocity + 0.000104 * velocity * velocity;
    let fraction =
        0.8 + 0.1894393 * (-0.012778 * minutes).exp() + 0.2989558 * (-0.1932605 * minutes).exp();
    oxygen / fraction
}

/// The class of each step (None outside any pass), and whether its pass counts.
fn step_classes(passes: &[Pass], len: usize) -> (Vec<Option<&'static str>>, Vec<bool>) {
    let mut classes = vec![None; len];
    let mut counted = vec![false; len];
    for pass in passes {
        for k in pass.start + 1..=pass.end {
            classes[k] = Some(pass.class);
            counted[k] = !pass.excluded;
        }
    }
    (classes, counted)
}

/// Efficiency lost from the first to the second half of the longest run.
fn decoupling(activity: &Activity, passes: &[Pass], steps: &[Step], t: &[f64]) -> Option<f64> {
    let pass = passes
        .iter()
        .filter(|p| p.class == RUN && !p.excluded)
        .max_by(|a, b| (t[a.end] - t[a.start]).total_cmp(&(t[b.end] - t[b.start])))?;
    if t[pass.end] - t[pass.start] < DECOUPLING_MIN_PASS_S {
        return None;
    }
    let begin = t[pass.start] + DECOUPLING_SKIP_S;
    let middle = f64::midpoint(begin, t[pass.end]);
    // (seconds with HR, metres, heartbeats-weighted seconds) per half.
    let mut halves = [(0.0, 0.0, 0.0); 2];
    for k in pass.start + 1..=pass.end {
        if t[k] <= begin {
            continue;
        }
        let Some(hr) = activity.points[k].hr else {
            continue;
        };
        let half = &mut halves[usize::from(t[k] > middle)];
        half.0 += steps[k].dt;
        half.1 += steps[k].dm;
        half.2 += hr * steps[k].dt;
    }
    let efficiency = |(time, metres, beats): (f64, f64, f64)| {
        (time > 0.0 && metres > 0.0 && beats > 0.0).then(|| metres / beats)
    };
    let first = efficiency(halves[0])?;
    let second = efficiency(halves[1])?;
    Some((first - second) / first * 100.0)
}

/// Median HR drop over the first minute of the walks that follow a run.
fn recovery(activity: &Activity, passes: &[Pass], t: &[f64]) -> Option<f64> {
    let points = &activity.points;
    let mut drops: Vec<f64> = passes
        .windows(2)
        .filter(|pair| {
            let (run, walk) = (&pair[0], &pair[1]);
            run.class == RUN
                && !run.excluded
                && walk.class == WALK
                && walk.start == run.end
                && t[run.end] - t[run.start] >= RECOVERY_MIN_RUN_S
                && t[walk.end] - t[walk.start] >= RECOVERY_WINDOW_S
        })
        .filter_map(|pair| {
            let at_stop = pair[0].end;
            let later =
                (at_stop..=pair[1].end).find(|&k| t[k] >= t[at_stop] + RECOVERY_WINDOW_S)?;
            Some(points[at_stop].hr? - points[later].hr?)
        })
        .collect();
    (!drops.is_empty()).then(|| median(&mut drops))
}

/// Pace of the run passes as if flat: each window's distance is scaled by its
/// Minetti cost, from the same smoothed altitude as the elevation stats.
fn grade_adjusted_pace(
    activity: &Activity,
    passes: &[Pass],
    steps: &[Step],
    t: &[f64],
    args: &Args,
) -> Option<f64> {
    let altitude = smoothed_altitude(&activity.points, t);
    let flat_cost = minetti_cost(0.0);
    let (mut total_time, mut flat_metres, mut has_grade) = (0.0, 0.0, false);
    for pass in passes.iter().filter(|p| p.class == RUN && !p.excluded) {
        let (mut from, mut time, mut metres) = (pass.start, 0.0, 0.0);
        for k in pass.start + 1..=pass.end {
            time += steps[k].dt;
            metres += steps[k].dm;
            if time < args.window as f64 && k < pass.end {
                continue;
            }
            let window_pace = (metres > 0.0).then(|| time / (metres / 1000.0));
            let guarded = window_pace.is_none_or(|pace| {
                args.max_pace.is_some_and(|max| pace > max as f64)
                    || args.min_pace.is_some_and(|min| pace < min as f64)
            });
            if !guarded {
                let grade = match (altitude[from], altitude[k]) {
                    (Some(a), Some(b)) => {
                        has_grade = true;
                        (b - a) / metres
                    }
                    _ => 0.0,
                };
                total_time += time;
                flat_metres += metres * minetti_cost(grade) / flat_cost;
            }
            from = k;
            time = 0.0;
            metres = 0.0;
        }
    }
    (has_grade && flat_metres > 0.0).then(|| total_time / (flat_metres / 1000.0))
}

/// The fastest contiguous `distance_m` with at least `min_run_share` running.
///
/// A pause longer than --gap ends the stretch, and a time faster than --min-pace
/// is taken for a GPS glitch.
fn best_effort(
    activity: &Activity,
    steps: &[Step],
    classes: &[Option<&'static str>],
    distance_m: f64,
    min_run_share: f64,
    args: &Args,
) -> Option<Effort> {
    let n = steps.len();
    let (mut time, mut metres, mut running) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for k in 1..n {
        time[k] = time[k - 1] + steps[k].dt;
        metres[k] = metres[k - 1] + steps[k].dm;
        running[k] = running[k - 1]
            + if classes[k] == Some(RUN) {
                steps[k].dt
            } else {
                0.0
            };
    }

    let mut best: Option<Effort> = None;
    let mut lo = 0;
    for hi in 1..n {
        if steps[hi].gap {
            lo = hi;
            continue;
        }
        while metres[hi] - metres[lo + 1] >= distance_m {
            lo += 1;
        }
        let span = metres[hi] - metres[lo];
        let elapsed = time[hi] - time[lo];
        if span < distance_m || elapsed <= 0.0 {
            continue;
        }
        let run_share = (running[hi] - running[lo]) / elapsed;
        // The window overshoots by up to one sample, so scale to the exact distance.
        let scaled = elapsed * distance_m / span;
        let pace = scaled / (distance_m / 1000.0);
        if run_share < min_run_share || args.min_pace.is_some_and(|min| pace < min as f64) {
            continue;
        }
        if best.as_ref().is_none_or(|b| scaled < b.time_s) {
            best = Some(Effort {
                distance_m,
                time_s: scaled,
                run_share,
                date: activity.date,
                activity_id: activity.activity_id.clone(),
            });
        }
    }
    best
}

fn session<'a>(
    activity: &'a Activity,
    route: Option<usize>,
    route_name: String,
    args: &Args,
) -> Session<'a> {
    let passes = build_passes(activity, args);
    let points = &activity.points;
    let steps = steps(points, args);
    let t = offsets(points);
    let (classes, counted) = step_classes(&passes, points.len());

    let of_class = |class: &str| -> Vec<&Pass> {
        passes
            .iter()
            .filter(|p| p.class == class && !p.excluded)
            .collect()
    };
    let (run_passes, walk_passes) = (of_class(RUN), of_class(WALK));
    let run = totals(&run_passes);
    let walk = totals(&walk_passes);
    let (run_s, run_m) = run
        .as_ref()
        .map_or((0.0, 0.0), |r| (r.time_s, r.distance_m));
    let (walk_s, walk_m) = walk
        .as_ref()
        .map_or((0.0, 0.0), |w| (w.time_s, w.distance_m));
    let run_hr = run.as_ref().and_then(|r| r.hr_mean);
    let longest = run_passes
        .iter()
        .max_by(|a, b| a.stats.time_s.total_cmp(&b.stats.time_s));

    let (mut zones_all, mut zones_run) = ([0.0; ZONES], [0.0; ZONES]);
    let (mut zone3_time, mut zone3_metres) = (0.0, 0.0);
    let hr_max = args.hr_max.map(|bpm| bpm as f64);
    if let Some(hr_max) = hr_max {
        for k in 1..points.len() {
            let Some(zone) = points[k].hr.and_then(|hr| hr_zone(hr, hr_max)) else {
                continue;
            };
            zones_all[zone] += steps[k].dt;
            if classes[k] == Some(RUN) && counted[k] {
                zones_run[zone] += steps[k].dt;
                if zone == 2 {
                    zone3_time += steps[k].dt;
                    zone3_metres += steps[k].dm;
                }
            }
        }
    }
    let has_hr = points.iter().any(|p| p.hr.is_some());

    let effort = |min_share: f64| -> Vec<Option<Effort>> {
        EFFORT_DISTANCES_M
            .iter()
            .map(|&d| best_effort(activity, &steps, &classes, d, min_share, args))
            .collect()
    };

    Session {
        activity,
        route,
        route_name,
        run_s,
        walk_s,
        run_m,
        walk_m,
        longest_run_s: longest.map_or(0.0, |p| p.stats.time_s),
        longest_run_m: longest.map_or(0.0, |p| p.stats.distance_m),
        run_hr,
        elev_gain: passes
            .iter()
            .filter_map(|p| p.stats.elev_gain)
            .reduce(|a, b| a + b),
        efficiency: run_hr
            .filter(|&hr| hr > 0.0 && run_s > 0.0)
            .map(|hr| (run_m / (run_s / 60.0)) / hr),
        decoupling: decoupling(activity, &passes, &steps, &t),
        zone3_pace: (zone3_time >= ZONE_PACE_MIN_S && zone3_metres > 0.0)
            .then(|| zone3_time / (zone3_metres / 1000.0)),
        recovery: recovery(activity, &passes, &t),
        grade_adjusted_pace: grade_adjusted_pace(activity, &passes, &steps, &t, args),
        zones_all,
        zones_run,
        trimp: (hr_max.is_some() && has_hr).then(|| trimp(&zones_all)),
        efforts_any: effort(0.0),
        efforts_running: effort(RUNNING_EFFORT_SHARE),
    }
}

/// The dense list of weeks from the first session to the last.
pub fn weeks(sessions: &[Session]) -> Vec<Week> {
    let (Some(first), Some(last)) = (
        sessions.iter().map(|s| s.activity.date).min(),
        sessions.iter().map(|s| s.activity.date).max(),
    ) else {
        return Vec::new();
    };
    let mut out: Vec<Week> = Vec::new();
    let mut monday = first.monday();
    while monday <= last {
        out.push(Week::empty(monday));
        monday = monday.plus_days(7);
    }
    for s in sessions {
        let index = ((s.activity.date.monday().days() - first.monday().days()) / 7) as usize;
        let week = &mut out[index];
        week.sessions += 1;
        if s.longest_run_s > week.longest_run_s {
            week.longest_run_s = s.longest_run_s;
            week.longest_run_m = s.longest_run_m;
        }
        week.run_s += s.run_s;
        week.walk_s += s.walk_s;
        week.km += s.km();
        for zone in 0..ZONES {
            week.zones_all[zone] += s.zones_all[zone];
            week.zones_run[zone] += s.zones_run[zone];
        }
        if let Some(value) = s.trimp {
            week.trimp = Some(week.trimp.unwrap_or(0.0) + value);
        }
    }
    let baselines: Vec<Option<f64>> = out
        .iter()
        .map(|w| previous_weeks(&out, w.monday).map(|b| b.km))
        .collect();
    for (week, baseline) in out.iter_mut().zip(baselines) {
        week.km_baseline = baseline;
    }
    out
}

/// The mean week of the four before `monday`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Baseline {
    pub sessions: f64,
    pub run_s: f64,
    pub km: f64,
    /// The longest run in those four weeks, not a mean.
    pub longest_run_s: f64,
    pub longest_run_m: f64,
}

/// None until the history covers all four weeks. A week without a row after the
/// last session is an empty week.
pub fn previous_weeks(weeks: &[Week], monday: Date) -> Option<Baseline> {
    let from = monday.minus_days(28);
    if weeks.first()?.monday > from {
        return None;
    }
    let mut baseline = Baseline {
        sessions: 0.0,
        run_s: 0.0,
        km: 0.0,
        longest_run_s: 0.0,
        longest_run_m: 0.0,
    };
    for week in weeks
        .iter()
        .filter(|w| w.monday >= from && w.monday < monday)
    {
        baseline.sessions += week.sessions as f64 / 4.0;
        baseline.run_s += week.run_s / 4.0;
        baseline.km += week.km / 4.0;
        if week.longest_run_s > baseline.longest_run_s {
            baseline.longest_run_s = week.longest_run_s;
            baseline.longest_run_m = week.longest_run_m;
        }
    }
    Some(baseline)
}

/// The last 7 days against the mean 7 days of the 28 before them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Load {
    pub acute: f64,
    pub baseline: f64,
    /// None with under 4 weeks of history or an empty baseline.
    pub ratio: Option<f64>,
}

/// The acute window is kept out of its own baseline, so a jump shows at full size.
pub fn acute_chronic(samples: &[(Date, f64)], reference: Date) -> Load {
    let age = |date: Date| reference.days() - date.days();
    let acute: f64 = samples
        .iter()
        .filter(|(d, _)| (0..7).contains(&age(*d)))
        .map(|x| x.1)
        .sum();
    let chronic: f64 = samples
        .iter()
        .filter(|(d, _)| (7..35).contains(&age(*d)))
        .map(|x| x.1)
        .sum();
    let baseline = chronic / 4.0;
    let enough = samples.iter().any(|(d, _)| age(*d) >= 34);
    Load {
        acute,
        baseline,
        ratio: (enough && baseline > 0.0).then(|| acute / baseline),
    }
}

/// Kilometres run, acute against chronic.
pub(crate) fn km_load(metrics: &Metrics, reference: Date) -> Load {
    let samples: Vec<(Date, f64)> = metrics
        .sessions
        .iter()
        .map(|s| (s.activity.date, s.km()))
        .collect();
    acute_chronic(&samples, reference)
}

/// TRIMP, acute against chronic, from the sessions with heart rate.
pub(crate) fn trimp_load(metrics: &Metrics, reference: Date) -> Load {
    let samples: Vec<(Date, f64)> = metrics
        .sessions
        .iter()
        .filter_map(|s| s.trimp.map(|value| (s.activity.date, value)))
        .collect();
    acute_chronic(&samples, reference)
}

/// The median elevation gain of these sessions.
pub(crate) fn typical_gain(sessions: &[&Session]) -> Option<f64> {
    upper_median(sessions.iter().filter_map(|s| s.elev_gain).collect())
}

/// The effort with the shortest time.
pub(crate) fn fastest<'e>(efforts: impl Iterator<Item = &'e Effort>) -> Option<&'e Effort> {
    efforts.min_by(|a, b| a.time_s.total_cmp(&b.time_s))
}

/// The effort predictions are made from: within the last 90 days, at least 3 km,
/// almost entirely running. The longest distance wins, then the fastest time.
pub fn prediction_source(sessions: &[Session], reference: Date) -> Option<Effort> {
    sessions
        .iter()
        .filter(|s| {
            (0..=PREDICTION_MAX_AGE_DAYS).contains(&(reference.days() - s.activity.date.days()))
        })
        .flat_map(|s| s.efforts_running.iter().flatten())
        .filter(|e| e.distance_m >= PREDICTION_MIN_DISTANCE_M)
        .min_by(|a, b| {
            b.distance_m
                .total_cmp(&a.distance_m)
                .then(a.time_s.total_cmp(&b.time_s))
        })
        .cloned()
}

/// Metrics for the activities up to --end, oldest first.
pub fn build<'a>(activities: &[&'a Activity], args: &Args) -> Metrics<'a> {
    let mut ordered: Vec<&Activity> = activities
        .iter()
        .copied()
        .filter(|a| !a.points.is_empty() && args.end.is_none_or(|end| a.date <= end))
        .collect();
    ordered.sort_by_key(|a| (a.date, a.points[0].time.instant()));

    let cells: Vec<BTreeSet<(i64, i64)>> = ordered.iter().map(|a| route_cells(a)).collect();
    let routes = assign_routes(&cells);
    // Each route is described by its first track, which the others were matched to.
    let mut names: Vec<String> = Vec::new();
    for (activity, route) in ordered.iter().zip(&routes) {
        if *route == Some(names.len()) {
            names.push(describe_route(names.len(), activity));
        }
    }
    let sessions: Vec<Session> = ordered
        .iter()
        .zip(routes)
        .map(|(activity, route)| {
            let name = route.map_or(NO_ROUTE.to_string(), |r| names[r].clone());
            session(activity, route, name, args)
        })
        .collect();
    let weeks = weeks(&sessions);
    Metrics { sessions, weeks }
}

/// The sessions inside --start/--end.
pub fn sessions_in_range<'m, 'a>(metrics: &'m Metrics<'a>, args: &Args) -> Vec<&'m Session<'a>> {
    metrics
        .sessions
        .iter()
        .filter(|s| in_range(s.activity, args))
        .collect()
}
