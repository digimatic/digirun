//! Console report.

use std::collections::BTreeMap;

use crate::analyze::stats;
use crate::args::{Args, Method, AMBIGUOUS_CADENCE};
use crate::model::{Activity, Segment, Stats};

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

pub fn print_report(
    activities: &[&Activity],
    segments: &[&Segment],
    args: &Args,
    skipped: &[String],
    notes: &[String],
) {
    let show_cad = args.cadence_stats;

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
    println!();

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

    if !skipped.is_empty() {
        println!();
        println!("Skipped (no trackpoints): {}", skipped.join(", "));
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
