//! Analyze downloaded GPX/TCX files: split each activity into walk and run segments.
//!
//! Reads the files already written by garmin-runs (no network access), splits each
//! activity into time windows, classifies every window as walk or run, and reports
//! mean pace plus standard deviation per class — per activity, per week, and pooled
//! overall.
//!
//! Classification is threshold-based: pace is derived from position deltas (GPX has
//! no speed field), cadence comes from the trackpoint extensions when present.

pub mod analyze;
pub mod args;
pub mod csv;
pub mod csvout;
pub mod dt;
pub mod load;
pub mod model;
pub mod parse;
pub mod passes;
pub mod report;
pub mod xml;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::args::{Args, Method};
use crate::model::{Activity, Segment};

/// Run the analysis and return the exit code.
pub fn run(args: &Args) -> i32 {
    let out_dir = expanduser(&args.out);
    if !out_dir.is_dir() {
        eprintln!(
            "error: {} is not a directory (run garmin-runs first)",
            out_dir.display()
        );
        return 1;
    }

    let loaded = load::load_activities(&out_dir, args.jobs);
    for error in &loaded.errors {
        eprintln!("error: {error}");
    }
    if loaded.activities.is_empty() {
        println!("No activities with trackpoints found.");
        return 0;
    }

    let mut activities = loaded.activities;
    let wanted = &args.types;
    let type_map = if wanted.is_some() {
        load::activity_types(&out_dir)
    } else {
        BTreeMap::new()
    };
    if wanted.is_some() && type_map.is_empty() {
        eprintln!(
            "warning: no summary.csv in {}, activity types cannot be checked",
            out_dir.display()
        );
    }
    for activity in &mut activities {
        activity.r#type = type_map
            .get(&activity.activity_id)
            .cloned()
            .unwrap_or_default();
    }

    let refs: Vec<&Activity> = activities.iter().collect();
    let (kept, type_skipped, type_unknown) = analyze::filter_activities(&refs, wanted);
    if kept.is_empty() {
        println!(
            "No activities left after the --type filter ({type_skipped} skipped, {type_unknown} unknown)."
        );
        return 0;
    }

    // --activity narrows every table; the date range is applied per table below.
    let kept: Vec<&Activity> = kept
        .into_iter()
        .filter(|a| {
            args.activities
                .as_ref()
                .is_none_or(|ids| ids.contains(&a.activity_id))
        })
        .collect();

    let mut segments: Vec<Segment> = Vec::new();
    let (mut dropped_slow, mut dropped_fast) = (0, 0);
    let mut kept_segments: Vec<&Segment> = Vec::new();
    if args.show.segments() {
        segments = collect_segments(&kept, args);
    }
    let seg_refs: Vec<&Segment> = segments.iter().collect();
    if args.show.segments() {
        (kept_segments, dropped_slow, dropped_fast) = analyze::filter_segments(&seg_refs, args);
        if kept_segments.is_empty() {
            println!("No segments matched — try a smaller --window or a wider date range.");
            if !args.show.detail {
                return 0;
            }
        }
    }

    let detail: Vec<(&Activity, Vec<passes::Pass>)> = if args.show.detail {
        kept.iter()
            .copied()
            .filter(|a| analyze::in_range(a, args))
            .map(|a| (a, passes::build_passes(a, args)))
            .collect()
    } else {
        Vec::new()
    };

    let mut notes: Vec<String> = Vec::new();
    let range = match (args.start, args.end) {
        (Some(s), Some(e)) => Some(format!("{s} to {e}")),
        (Some(s), None) => Some(format!("from {s}")),
        (None, Some(e)) => Some(format!("up to {e}")),
        (None, None) => None,
    };
    if let Some(mut range) = range {
        if let Some(days) = args.days {
            range.push_str(&format!(" (last {days} days)"));
        }
        notes.push(format!("Range: {range}"));
    }
    if let Some(ids) = &args.activities {
        notes.push(format!(
            "Activities: {}",
            ids.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if let Some(wanted) = wanted {
        let types = wanted.iter().cloned().collect::<Vec<_>>().join(", ");
        let mut parts: Vec<String> = Vec::new();
        if type_skipped > 0 {
            parts.push(format!("{type_skipped} skipped"));
        }
        if type_unknown > 0 {
            parts.push(format!("{type_unknown} skipped, type unknown"));
        }
        let mut note = format!("Types: {types}");
        if !parts.is_empty() {
            note.push_str(&format!(" ({})", parts.join(", ")));
        }
        notes.push(note);
    }
    if let Some(max) = args.max_pace {
        let mut note = format!(
            "Max pace: slower than {}/km excluded",
            report::fmt_pace(max as f64)
        );
        if args.show.segments() {
            note.push_str(&format!(" ({dropped_slow} segments)"));
        }
        notes.push(note);
    }
    if let Some(min) = args.min_pace {
        let mut note = format!(
            "Min pace: faster than {}/km excluded",
            report::fmt_pace(min as f64)
        );
        if args.show.segments() {
            note.push_str(&format!(" ({dropped_fast} segments)"));
        }
        notes.push(note);
    }
    if args.cadence_stats && args.show.segments() {
        notes.push("Cadence columns are mean + stdev in strides/min".to_string());
    }
    if args.show.detail {
        notes.push(format!(
            "Passes: cadence median over {}s, passes under {}s join a neighbour",
            args.smooth, args.min_pass
        ));
    }

    report::print_header(args, &notes);
    if !kept_segments.is_empty() {
        report::print_report(&kept, &kept_segments, args);
    }
    if args.show.detail {
        if detail.is_empty() {
            println!();
            println!("No activities in the selected range for the detail.");
        } else {
            report::print_detail(&detail);
        }
    }
    if !loaded.skipped.is_empty() {
        println!();
        println!("Skipped (no trackpoints): {}", loaded.skipped.join(", "));
    }

    if !args.no_csv {
        let report_dir = expanduser(&args.report_dir);
        if !kept_segments.is_empty() {
            if let Err(error) = csvout::write_csvs(&report_dir, &kept, &kept_segments, args) {
                eprintln!("error: {error}");
                return 1;
            }
        }
        if !detail.is_empty() {
            if let Err(error) = csvout::write_passes(&report_dir, &detail) {
                eprintln!("error: {error}");
                return 1;
            }
        }
    }
    0
}

fn collect_segments(activities: &[&Activity], args: &Args) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for activity in activities {
        if !analyze::in_range(*activity, args) {
            continue;
        }
        if args.method == Method::Lap {
            segments.extend(analyze::lap_segments(*activity, args));
        } else {
            segments.extend(analyze::build_segments(*activity, args));
        }
    }
    segments
}

fn expanduser(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut out = PathBuf::new();
    if components.peek() == Some(&std::path::Component::Normal(std::ffi::OsStr::new("~"))) {
        if let Ok(home) = std::env::var("HOME") {
            out.push(home);
            components.next();
        }
    }
    // pathlib drops the "." components, so "./runs" displays as "runs".
    for part in components {
        match part {
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        return PathBuf::from(".");
    }
    out
}
