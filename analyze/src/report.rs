//! Console report.

use std::collections::BTreeMap;

use crate::args::{AMBIGUOUS_CADENCE, Args, Method};
use crate::dt::Date;
use crate::format::{change, duration_or_dash, fmt_duration, fmt_pace, py_num};
use crate::metrics::{
    EFFORT_DISTANCES_M, Effort, Load, Metrics, RUNNING_EFFORT_SHARE, Session, ZONES, fastest,
    km_load, prediction_source, previous_weeks, riegel, sessions_in_range, trimp_load,
    typical_gain, vdot,
};
use crate::model::{Activity, Segment, Stats};
use crate::num::{mean_of, round_int};
use crate::passes::{Pass, Source, totals};
use crate::segments::{RUN, WALK, of_class, stats};

/// Right-aligned cell, or `-` when the value is missing.
fn cell(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{:>6}", py_num(value)),
        None => "     -".to_string(),
    }
}

fn cells(st: &Stats, show_cad: bool) -> Vec<String> {
    let mut out = vec![
        format!("{:>6}", st.segments),
        format!("{:>7}", fmt_pace(st.mean)),
        format!("{:>6}", fmt_pace(st.stdev)),
    ];
    if show_cad {
        out.push(cell(st.cad_mean));
        out.push(cell(st.cad_stdev));
    }
    out
}

fn table_header(leading: &[String], show_cad: bool) -> String {
    let mut parts = leading.to_vec();
    parts.push(format!("{:>6}", "run n"));
    parts.push(format!("{:>7}", "run"));
    parts.push(format!("{:>6}", "sd"));
    if show_cad {
        parts.push(format!("{:>6}", "cad"));
        parts.push(format!("{:>6}", "sd"));
    }
    parts.push(format!("{:>6}", "walk n"));
    parts.push(format!("{:>7}", "walk"));
    parts.push(format!("{:>6}", "sd"));
    if show_cad {
        parts.push(format!("{:>6}", "cad"));
        parts.push(format!("{:>6}", "sd"));
    }
    parts.join(" ")
}

pub fn method_note(args: &Args) -> String {
    let pace = fmt_pace(args.pace_threshold as f64);
    match args.method {
        Method::Cadence => format!(
            "cadence >= {} strides/min (pace fallback: {}/km)",
            args.cadence_threshold, pace
        ),
        Method::Pace => format!("pace <= {}/km", pace),
        Method::Lap => format!("TCX laps, each labelled by pace <= {}/km", pace),
        Method::Hybrid => format!(
            "cadence < {lo} walk / > {hi} run, band {lo}-{hi} resolved by pace <= {pace}/km",
            lo = AMBIGUOUS_CADENCE.0,
            hi = AMBIGUOUS_CADENCE.1,
            pace = pace
        ),
    }
}

/// The method and window lines, then the notes (filters, range, ...).
pub fn print_header(args: &Args, notes: &[String]) {
    println!("Method: {} — {}", args.method.as_str(), method_note(args));
    println!(
        "Window: {}s, min segment {}s / {}m, gap {}s",
        args.window,
        args.min_segment,
        py_num(args.min_distance),
        args.gap
    );
    for note in notes {
        println!("{note}");
    }
}

/// The tables built from the segments, as chosen with --show.
pub fn print_report(activities: &[&Activity], segments: &[&Segment], args: &Args) {
    let show_cad = args.cadence_stats;

    // Each table starts with its own blank line, so any subset of them is spaced right.
    if args.show.activities {
        print_activities(activities, segments, show_cad);
    }
    if args.show.weekly {
        print_weekly(segments, show_cad);
    }
    if args.show.overall {
        print_overall(segments, show_cad);
    }
}

fn print_activities(activities: &[&Activity], segments: &[&Segment], show_cad: bool) {
    let mut by_activity: BTreeMap<&str, Vec<&Segment>> = BTreeMap::new();
    for s in segments {
        by_activity
            .entry(s.activity_id.as_str())
            .or_default()
            .push(s);
    }

    let header = table_header(
        &[
            format!("{:<11}", "date"),
            format!("{:<8}", "type"),
            format!("{:<34}", "activity"),
        ],
        show_cad,
    );
    println!();
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    for activity in activities {
        let rows = match by_activity.get(activity.activity_id.as_str()) {
            Some(rows) => rows,
            None => continue,
        };
        if rows.is_empty() {
            continue;
        }
        let run = stats(&of_class(rows, RUN));
        let walk = stats(&of_class(rows, WALK));
        let mut parts = vec![
            format!("{:<11}", activity.date.isoformat()),
            format!(
                "{:<8}",
                if activity.r#type.is_empty() {
                    "-"
                } else {
                    &activity.r#type
                }
            ),
            format!("{:<34}", truncate(&activity.name, 34)),
        ];
        parts.extend(cells(&run, show_cad));
        parts.extend(cells(&walk, show_cad));
        println!("{}", parts.join(" "));
    }
}

fn print_weekly(segments: &[&Segment], show_cad: bool) {
    println!();
    println!("Weekly");
    let header = table_header(&[format!("{:<10}", "week")], show_cad);
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    let mut weeks: BTreeMap<String, Vec<&Segment>> = BTreeMap::new();
    for s in segments {
        let (year, week) = s.date.isocalendar();
        weeks
            .entry(format!("{year}-W{week:02}"))
            .or_default()
            .push(s);
    }
    for (week, rows) in &weeks {
        let run = stats(&of_class(rows, RUN));
        let walk = stats(&of_class(rows, WALK));
        let mut parts = vec![format!("{:<10}", week)];
        parts.extend(cells(&run, show_cad));
        parts.extend(cells(&walk, show_cad));
        println!("{}", parts.join(" "));
    }
}

fn print_overall(segments: &[&Segment], show_cad: bool) {
    println!();
    println!("Overall");
    let mut header = format!(
        "{:<6} {:>6} {:>7} {:>8} {:>7} {:>6} {:>7} {:>7} {:>6}",
        "class", "n", "time", "dist km", "mean", "sd", "min", "max", "share"
    );
    if show_cad {
        header.push_str(&format!(" {:>6} {:>6}", "cad", "sd"));
    }
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));

    let total_time: f64 = segments.iter().map(|s| s.duration_s).sum();
    for class in [RUN, WALK] {
        let rows = of_class(segments, class);
        if rows.is_empty() {
            continue;
        }
        let st = stats(&rows);
        let share = if total_time > 0.0 {
            st.time_s / total_time * 100.0
        } else {
            0.0
        };
        let mut line = format!(
            "{:<6} {:>6} {:>7} {:>8.2} {:>7} {:>6} {:>7} {:>7} {:>5.1}%",
            class,
            st.segments,
            fmt_duration(st.time_s),
            st.distance_km,
            fmt_pace(st.mean),
            fmt_pace(st.stdev),
            fmt_pace(st.min),
            fmt_pace(st.max),
            share
        );
        if show_cad {
            line.push_str(&format!(" {} {}", cell(st.cad_mean), cell(st.cad_stdev)));
        }
        println!("{line}");
    }
}

/// Right-aligned cell of `width`, or `-` when the value is missing.
fn opt(value: Option<f64>, width: usize, show: fn(f64) -> String) -> String {
    format!(
        "{:>width$}",
        value.map(show).unwrap_or_else(|| "-".to_string())
    )
}

fn int(value: f64) -> String {
    round_int(value).to_string()
}

fn one(value: f64) -> String {
    format!("{value:.1}")
}

fn pace(value: f64) -> String {
    fmt_pace(value)
}

/// One line per class: passes, time, distance, pace, cadence and HR.
fn totals_line(label: &str, passes: &[&Pass]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for class in [RUN, WALK] {
        let group: Vec<&Pass> = passes
            .iter()
            .copied()
            .filter(|p| p.class == class && !p.excluded)
            .collect();
        let Some(t) = totals(&group) else {
            continue;
        };
        let mut part = format!(
            "{class}: {} passes {} {:.2} km",
            t.passes,
            fmt_duration(t.time_s),
            t.distance_m / 1000.0
        );
        if let Some(p) = t.pace {
            part.push_str(&format!(" {}/km", fmt_pace(p)));
        }
        if let Some(c) = t.cad_mean {
            part.push_str(&format!(" cad {c:.1}"));
        }
        if let Some(hr) = t.hr_mean {
            part.push_str(&format!(" hr {}", int(hr)));
        }
        parts.push(part);
    }
    format!("  {label:<8} {}", parts.join(" | "))
}

/// One block per session with its passes, then the range summary.
pub fn print_detail(activities: &[(&Activity, Vec<Pass>)]) {
    for (activity, passes) in activities {
        print_session_passes(activity, passes);
    }
    print_range_totals(activities);
}

/// One activity's passes, then their totals.
fn print_session_passes(activity: &Activity, passes: &[Pass]) {
    let header = format!(
        "  {:>3} {:<6} {:>7} {:>7} {:>6} {:>6} {:>5} {:>6} {:>6} {:>5} {:>4} {:>4} {:>4} {:>4} {:>4} {:>4} {:>5} {:>5} {:>5} {:>6} {:>5}  laps",
        "#",
        "class",
        "start",
        "time",
        "dist m",
        "pace",
        "sd",
        "min",
        "max",
        "cad",
        "sd",
        "min",
        "max",
        "hr",
        "min",
        "max",
        "drift",
        "elev+",
        "elev-",
        "stride",
        "stop"
    );
    println!();
    println!(
        "{}  {}  {}  ({})",
        activity.date.isoformat(),
        if activity.r#type.is_empty() {
            "-"
        } else {
            &activity.r#type
        },
        activity.name,
        activity.activity_id
    );
    if passes.is_empty() {
        println!("  no passes");
        return;
    }
    if passes.iter().any(|p| p.source == Source::Pace) {
        println!("  no cadence in this activity: passes are split on pace");
    }
    println!("{header}");
    println!("  {}", "-".repeat(header.chars().count() - 2));
    for (i, p) in passes.iter().enumerate() {
        let s = &p.stats;
        let class = if p.excluded {
            format!("{}*", p.class)
        } else {
            p.class.to_string()
        };
        let laps = p
            .laps
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "  {:>3} {:<6} {:>7} {:>7} {:>6} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {:>5}  {}",
            i + 1,
            class,
            fmt_duration(s.start_offset_s),
            fmt_duration(s.time_s),
            int(s.distance_m),
            opt(s.pace.filter(|_| s.pace_reliable), 6, pace),
            opt(s.pace_sd, 5, pace),
            opt(s.pace_min, 6, pace),
            opt(s.pace_max, 6, pace),
            opt(s.cad_mean, 5, one),
            opt(s.cad_sd, 4, one),
            opt(s.cad_min, 4, int),
            opt(s.cad_max, 4, int),
            opt(s.hr_mean, 4, int),
            opt(s.hr_min, 4, int),
            opt(s.hr_max, 4, int),
            opt(s.hr_drift, 5, |v| format!("{:+}", round_int(v))),
            opt(s.elev_gain, 5, one),
            opt(s.elev_loss, 5, one),
            opt(s.stride_m, 6, |v| format!("{v:.2}")),
            duration_or_dash(s.stopped_s),
            laps
        );
    }
    let refs: Vec<&Pass> = passes.iter().collect();
    println!("{}", totals_line("Session", &refs));
}

/// Run and walk totals over every pass in the range.
fn print_range_totals(activities: &[(&Activity, Vec<Pass>)]) {
    let all: Vec<&Pass> = activities.iter().flat_map(|(_, p)| p.iter()).collect();
    println!();
    println!("Range ({} sessions)", activities.len());
    let header = format!(
        "{:<6} {:>6} {:>8} {:>8} {:>7} {:>6} {:>7} {:>7} {:>6} {:>5}",
        "class", "passes", "time", "dist km", "pace", "sd", "mean", "median", "cad", "hr"
    );
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    for class in [RUN, WALK] {
        let group: Vec<&Pass> = all
            .iter()
            .copied()
            .filter(|p| p.class == class && !p.excluded)
            .collect();
        let Some(t) = totals(&group) else {
            continue;
        };
        println!(
            "{:<6} {:>6} {:>8} {:>8.2} {} {} {:>7} {:>7} {} {}",
            class,
            t.passes,
            fmt_duration(t.time_s),
            t.distance_m / 1000.0,
            opt(t.pace, 7, pace),
            opt(t.pace_sd, 6, pace),
            fmt_duration(t.mean_pass_s),
            fmt_duration(t.median_pass_s),
            opt(t.cad_mean, 6, one),
            opt(t.hr_mean, 5, int)
        );
    }
    let excluded = all.iter().filter(|p| p.excluded).count();
    if excluded > 0 {
        println!();
        println!("* {excluded} passes outside --max-pace/--min-pace, left out of the totals");
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// The metric tables chosen with --show, for the range in `args`.
pub fn print_metrics(metrics: &Metrics, args: &Args, reference: Date) {
    if args.show.progress {
        print_progress(metrics, args);
    }
    if args.show.zones {
        print_zones(metrics, args, reference);
    }
    if args.show.fitness {
        print_fitness(metrics, args);
    }
    if args.show.efforts {
        print_efforts(metrics, args, reference);
    }
    if args.show.summary {
        print_summary(metrics, args, reference);
    }
}

fn print_progress(metrics: &Metrics, args: &Args) {
    println!();
    println!("Progress");
    let header = format!(
        "{:<10} {:>4} {:>8} {:>7} {:>8} {:>8} {:>5} {:>7} {:>6}",
        "week", "sess", "longest", "lng km", "run", "walk", "run%", "km", "vs 4wk"
    );
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    for week in metrics.weeks.iter().filter(|w| w.in_range(args)) {
        println!(
            "{:<10} {:>4} {:>8} {} {:>8} {:>8} {} {:>7.2} {:>6}",
            week.label(),
            week.sessions,
            duration_or_dash(week.longest_run_s),
            opt(
                (week.longest_run_m > 0.0).then_some(week.longest_run_m / 1000.0),
                7,
                |v| format!("{v:.2}")
            ),
            duration_or_dash(week.run_s),
            duration_or_dash(week.walk_s),
            opt(week.run_share(), 5, int),
            week.km,
            change(week.km, week.km_baseline)
        );
    }
}

fn load_line(label: &str, load: Load, unit: &str) -> String {
    format!(
        "{label:<7} last 7 days {:.1} {unit}, previous 4 weeks {:.1} {unit}/week, ratio {}",
        load.acute,
        load.baseline,
        load.ratio
            .map_or("n/a".to_string(), |ratio| format!("{ratio:.2}"))
    )
}

fn print_zones(metrics: &Metrics, args: &Args, reference: Date) {
    println!();
    println!("HR zones (minutes)");
    let Some(hr_max) = args.hr_max else {
        println!(
            "needs --hr-max: it is not guessed, because a sensor spike would shift every zone"
        );
        return;
    };
    println!("zones are 50-60, 60-70, 70-80, 80-90 and 90+% of {hr_max} bpm");
    let header = format!(
        "{:<10} {:>4} {:>4} {:>4} {:>4} {:>4}   {:>4} {:>4} {:>4} {:>4} {:>4} {:>6}",
        "week", "run1", "2", "3", "4", "5", "all1", "2", "3", "4", "5", "trimp"
    );
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    let minutes = |zones: &[f64; ZONES]| -> Vec<String> {
        zones
            .iter()
            .map(|s| format!("{:>4}", round_int(s / 60.0)))
            .collect()
    };
    for week in metrics.weeks.iter().filter(|w| w.in_range(args)) {
        println!(
            "{:<10} {}   {} {}",
            week.label(),
            minutes(&week.zones_run).join(" "),
            minutes(&week.zones_all).join(" "),
            opt(week.trimp, 6, int)
        );
    }
    println!();
    println!(
        "{}",
        load_line("TRIMP", trimp_load(metrics, reference), "pts")
    );
    println!("{}", load_line("Volume", km_load(metrics, reference), "km"));
}

fn print_fitness(metrics: &Metrics, args: &Args) {
    println!();
    println!("Fitness, by route");
    let sessions: Vec<&Session> = sessions_in_range(metrics, args)
        .into_iter()
        .filter(|s| s.run_s > 0.0)
        .collect();
    let mut routes: Vec<Option<usize>> = sessions.iter().map(|s| s.route).collect();
    routes.sort();
    routes.dedup();
    let header = format!(
        "  {:<10} {:>7} {:>6} {:>6} {:>6} {:>4} {:>5} {:>7} {:>6} {:>6} {:>5}",
        "date", "run", "km", "pace", "gap", "hr", "ef", "decoup%", "z3", "recov", "elev+"
    );
    for route in routes {
        let rows: Vec<&&Session> = sessions.iter().filter(|s| s.route == route).collect();
        let plain: Vec<&Session> = rows.iter().map(|s| **s).collect();
        let gain = typical_gain(&plain);
        println!();
        println!(
            "{} ({} sessions, typical gain {})",
            rows[0].route_name,
            rows.len(),
            gain.map_or("-".to_string(), |g| format!("{} m", round_int(g)))
        );
        println!("{header}");
        for s in &rows {
            println!(
                "  {:<10} {:>7} {:>6.2} {} {} {} {} {} {} {} {}",
                s.activity.date.isoformat(),
                fmt_duration(s.run_s),
                s.run_m / 1000.0,
                opt(s.run_pace(), 6, pace),
                opt(s.grade_adjusted_pace, 6, pace),
                opt(s.run_hr, 4, int),
                opt(s.efficiency, 5, |v| format!("{v:.2}")),
                opt(s.decoupling, 7, one),
                opt(s.zone3_pace, 6, pace),
                opt(s.recovery, 6, int),
                opt(s.elev_gain, 5, int),
            );
        }
        let efficiency: Vec<f64> = rows.iter().filter_map(|s| s.efficiency).collect();
        if efficiency.len() >= 6 {
            let early = mean_of(&efficiency[..3]).unwrap_or(0.0);
            let late = mean_of(&efficiency[efficiency.len() - 3..]).unwrap_or(0.0);
            println!("  efficiency, first 3 sessions {early:.2}, last 3 sessions {late:.2}");
        }
    }
}

fn effort_cells(effort: Option<&Effort>) -> String {
    match effort {
        Some(e) => format!(
            "{:>8} {:>6} {:<10} {:>4}%",
            fmt_duration(e.time_s),
            fmt_pace(e.pace()),
            e.date.isoformat(),
            round_int(e.run_share * 100.0)
        ),
        None => format!("{:>8} {:>6} {:<10} {:>5}", "-", "-", "-", "-"),
    }
}

fn print_efforts(metrics: &Metrics, args: &Args, reference: Date) {
    println!();
    println!("Best efforts");
    let share = round_int(RUNNING_EFFORT_SHARE * 100.0);
    let header = format!(
        "{:<5} {:>8} {:>6} {:<10} {:>5}   {:>8} {:>6} {:<10} {:>5}",
        "dist",
        "any",
        "pace",
        "date",
        "run",
        format!("{share}%+run"),
        "pace",
        "date",
        "run"
    );
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    let sessions = sessions_in_range(metrics, args);
    for (i, distance) in EFFORT_DISTANCES_M.iter().enumerate() {
        let any = fastest(sessions.iter().filter_map(|s| s.efforts_any[i].as_ref()));
        let running = fastest(
            sessions
                .iter()
                .filter_map(|s| s.efforts_running[i].as_ref()),
        );
        println!(
            "{:<5} {}   {}",
            format!("{} km", round_int(distance / 1000.0)),
            effort_cells(any),
            effort_cells(running)
        );
    }

    println!();
    match prediction_source(&metrics.sessions, reference) {
        Some(source) => {
            println!(
                "Predictions from {:.0} km in {} on {} (a training effort, so likely slower than a race):",
                source.distance_m / 1000.0,
                fmt_duration(source.time_s),
                source.date
            );
            println!("  VDOT {:.1}", vdot(source.distance_m, source.time_s));
            for target in [5000.0, 10000.0] {
                println!(
                    "  {:.0} km in {}",
                    target / 1000.0,
                    fmt_duration(riegel(source.time_s, source.distance_m, target))
                );
            }
        }
        None => println!(
            "No predictions: they need an effort of at least 3 km, {share}% running, in the last 90 days."
        ),
    }
}

fn print_summary(metrics: &Metrics, args: &Args, reference: Date) {
    let monday = reference.monday();
    let current = metrics.weeks.iter().find(|w| w.monday == monday);
    let baseline = previous_weeks(&metrics.weeks, monday);
    let (year, week) = monday.isocalendar();

    println!();
    println!("This week so far ({year}-W{week:02}) against the mean of the previous 4 weeks");
    let header = format!("{:<12} {:>8} {:>8} {:>6}", "", "week", "4 weeks", "change");
    println!("{header}");
    println!("{}", "-".repeat(header.chars().count()));
    let sessions = current.map_or(0, |w| w.sessions) as f64;
    let km = current.map_or(0.0, |w| w.km);
    let run_s = current.map_or(0.0, |w| w.run_s);
    let longest = current.map_or(0.0, |w| w.longest_run_s);
    let longest_km = current.map_or(0.0, |w| w.longest_run_m) / 1000.0;
    let baseline_longest_km = baseline.map(|b| b.longest_run_m / 1000.0);
    println!(
        "{:<12} {:>8} {} {:>6}",
        "sessions",
        round_int(sessions),
        opt(baseline.map(|b| b.sessions), 8, one),
        change(sessions, baseline.map(|b| b.sessions))
    );
    println!(
        "{:<12} {:>8.2} {} {:>6}",
        "km",
        km,
        opt(baseline.map(|b| b.km), 8, |v| format!("{v:.2}")),
        change(km, baseline.map(|b| b.km))
    );
    println!(
        "{:<12} {:>8} {} {:>6}",
        "run time",
        fmt_duration(run_s),
        opt(baseline.map(|b| b.run_s), 8, fmt_duration),
        change(run_s, baseline.map(|b| b.run_s))
    );
    println!(
        "{:<12} {:>8} {} {:>6}",
        "longest run",
        fmt_duration(longest),
        opt(baseline.map(|b| b.longest_run_s), 8, fmt_duration),
        change(longest, baseline.map(|b| b.longest_run_s))
    );
    println!(
        "{:<12} {:>8.2} {} {:>6}",
        "longest km",
        longest_km,
        opt(baseline_longest_km, 8, |v| format!("{v:.2}")),
        change(longest_km, baseline_longest_km)
    );
    println!("(the longest rows compare with the longest of those 4 weeks, not their mean)");
    println!();
    println!("{}", load_line("Volume", km_load(metrics, reference), "km"));
    if args.hr_max.is_some() {
        println!(
            "{}",
            load_line("TRIMP", trimp_load(metrics, reference), "pts")
        );
    }
}
