//! Console report.

use std::collections::BTreeMap;

use crate::analyze::stats;
use crate::analyze::{RUN, WALK};
use crate::args::{Args, Method, AMBIGUOUS_CADENCE};
use crate::model::{Activity, Segment, Stats};
use crate::passes::{totals, Pass, Source};

/// Python's `str(float)`: whole numbers keep the `.0`.
pub fn py_num(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if value.fract() == 0.0 {
        format!("{:.1}", value)
    } else {
        format!("{}", value)
    }
}

/// Python's `round(value)` to an integer (half to even).
fn round_int(value: f64) -> i64 {
    format!("{:.0}", value).parse::<i64>().unwrap_or(0)
}

/// Pace as `M:SS` per kilometre.
pub fn fmt_pace(seconds: f64) -> String {
    let total = round_int(seconds);
    format!("{}:{:02}", total / 60, total % 60)
}

/// Total time as `H:MM:SS`, or `M:SS` under an hour.
pub fn fmt_duration(seconds: f64) -> String {
    let total = round_int(seconds);
    let (h, rem) = (total / 3600, total % 3600);
    if h != 0 {
        format!("{h}:{:02}:{:02}", rem / 60, rem % 60)
    } else {
        format!("{}:{:02}", rem / 60, rem % 60)
    }
}

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
        let run = stats(
            &rows
                .iter()
                .filter(|s| s.class == "run")
                .copied()
                .collect::<Vec<_>>(),
        );
        let walk = stats(
            &rows
                .iter()
                .filter(|s| s.class == "walk")
                .copied()
                .collect::<Vec<_>>(),
        );
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
        let run = stats(
            &rows
                .iter()
                .filter(|s| s.class == "run")
                .copied()
                .collect::<Vec<_>>(),
        );
        let walk = stats(
            &rows
                .iter()
                .filter(|s| s.class == "walk")
                .copied()
                .collect::<Vec<_>>(),
        );
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
    for class in ["run", "walk"] {
        let rows: Vec<&Segment> = segments
            .iter()
            .copied()
            .filter(|s| s.class == class)
            .collect();
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
    let header = format!(
        "  {:>3} {:<6} {:>7} {:>7} {:>6} {:>6} {:>5} {:>6} {:>6} {:>5} {:>4} {:>4} {:>4} {:>4} {:>4} {:>4} {:>5} {:>5} {:>5} {:>6} {:>5}  laps",
        "#", "class", "start", "time", "dist m", "pace", "sd", "min", "max", "cad", "sd", "min",
        "max", "hr", "min", "max", "drift", "elev+", "elev-", "stride", "stop"
    );
    let mut excluded = 0usize;
    for (activity, passes) in activities {
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
            continue;
        }
        if passes.iter().any(|p| p.source == Source::Pace) {
            println!("  no cadence in this activity: passes are split on pace");
        }
        println!("{header}");
        println!("  {}", "-".repeat(header.chars().count() - 2));
        for (i, p) in passes.iter().enumerate() {
            let s = &p.stats;
            excluded += p.excluded as usize;
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
                opt(s.pace, 6, pace),
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
                if s.stopped_s > 0.0 {
                    fmt_duration(s.stopped_s)
                } else {
                    "-".to_string()
                },
                laps
            );
        }
        let refs: Vec<&Pass> = passes.iter().collect();
        println!("{}", totals_line("Session", &refs));
    }

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
    if excluded > 0 {
        println!();
        println!("* {excluded} passes outside --max-pace/--min-pace, left out of the totals");
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
