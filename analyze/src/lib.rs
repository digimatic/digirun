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
pub mod format;
pub mod html;
pub mod load;
pub mod map;
pub mod metrics;
pub mod model;
pub mod naming;
pub mod num;
pub mod parse;
pub mod passes;
pub mod report;
pub mod routes;
pub mod serve;
pub mod xml;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::args::{Args, Method};
use crate::model::{Activity, Segment};

/// Run the analysis and return the exit code.
pub fn run(args: &Args) -> i32 {
    match try_run(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

/// What the filters left out, for the notes above the tables.
#[derive(Default)]
struct Dropped {
    type_skipped: usize,
    type_unknown: usize,
    slow_segments: usize,
    fast_segments: usize,
}

fn try_run(args: &Args) -> Result<i32, String> {
    let out_dir = expanduser(&args.out);
    if !out_dir.is_dir() {
        return Err(format!(
            "{} is not a directory (run garmin-runs first)",
            out_dir.display()
        ));
    }

    let loaded = load::load_activities(&out_dir, args.jobs);
    for error in &loaded.errors {
        eprintln!("error: {error}");
    }
    if loaded.activities.is_empty() {
        println!("No activities with trackpoints found.");
        return Ok(0);
    }
    let mut activities = loaded.activities;
    set_types(&mut activities, args, &out_dir);
    let loaded_ids: BTreeSet<&str> = activities.iter().map(|a| a.activity_id.as_str()).collect();

    let refs: Vec<&Activity> = activities.iter().collect();
    let (kept, type_skipped, type_unknown) = analyze::filter_activities(&refs, &args.types);
    if kept.is_empty() {
        println!(
            "No activities left after the --type filter ({type_skipped} skipped, {type_unknown} unknown)."
        );
        return Ok(0);
    }
    let mut dropped = Dropped {
        type_skipped,
        type_unknown,
        ..Dropped::default()
    };

    // --activity narrows every table; the date range is applied per table below.
    let kept: Vec<&Activity> = kept
        .into_iter()
        .filter(|a| {
            args.activities
                .as_ref()
                .is_none_or(|ids| ids.contains(&a.activity_id))
        })
        .collect();
    let reference = args.end.unwrap_or_else(dt::Date::today);

    if args.name_routes || args.serve {
        let mut metrics = named_metrics(&kept, args, &out_dir, &loaded_ids)?;
        let path = route_names_path(args, &out_dir);
        if args.name_routes {
            return Ok(naming::run(&metrics, args, &path));
        }
        return Ok(serve::run(&mut serve::State::new(
            &mut metrics,
            args,
            path,
            reference,
        )));
    }

    let segments: Vec<Segment> = if args.show.segments() {
        collect_segments(&kept, args)
    } else {
        Vec::new()
    };
    let seg_refs: Vec<&Segment> = segments.iter().collect();
    let mut kept_segments: Vec<&Segment> = Vec::new();
    if args.show.segments() {
        (kept_segments, dropped.slow_segments, dropped.fast_segments) =
            analyze::filter_segments(&seg_refs, args);
        if kept_segments.is_empty() {
            println!("No segments matched — try a smaller --window or a wider date range.");
            if !args.show.detail && args.html.is_none() {
                return Ok(0);
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

    let metrics = if args.show.metrics() || args.html.is_some() {
        Some(named_metrics(&kept, args, &out_dir, &loaded_ids)?)
    } else {
        None
    };

    report::print_header(args, &notes(args, &dropped));
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
    if let Some(metrics) = &metrics {
        report::print_metrics(metrics, args, reference);
    }
    if !loaded.skipped.is_empty() {
        println!();
        println!("Skipped (no trackpoints): {}", loaded.skipped.join(", "));
    }

    if !args.no_csv {
        write_csvs(args, &kept, &kept_segments, &detail, metrics.as_ref())?;
    }
    if let (Some(path), Some(metrics)) = (&args.html, &metrics) {
        let path = expanduser(path);
        html::write_report(&path, metrics, args, reference)?;
        println!();
        println!("Wrote {}", path.display());
    }
    Ok(0)
}

/// Give each activity its type from summary.csv, when --type needs them.
fn set_types(activities: &mut [Activity], args: &Args, out_dir: &Path) {
    if args.types.is_none() {
        return;
    }
    let type_map = load::activity_types(out_dir);
    if type_map.is_empty() {
        eprintln!(
            "warning: no summary.csv in {}, activity types cannot be checked",
            out_dir.display()
        );
    }
    for activity in activities {
        activity.r#type = type_map
            .get(&activity.activity_id)
            .cloned()
            .unwrap_or_default();
    }
}

/// The metrics, with the routes named from routes.txt.
fn named_metrics<'a>(
    kept: &[&'a Activity],
    args: &Args,
    out_dir: &Path,
    loaded_ids: &BTreeSet<&str>,
) -> Result<metrics::Metrics<'a>, String> {
    let mut metrics = metrics::build(kept, args);
    apply_route_names(&mut metrics, args, out_dir, loaded_ids)?;
    Ok(metrics)
}

/// The lines above the tables that say which filters and settings apply.
fn notes(args: &Args, dropped: &Dropped) -> Vec<String> {
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
    if let Some(wanted) = &args.types {
        let types = wanted.iter().cloned().collect::<Vec<_>>().join(", ");
        let mut parts: Vec<String> = Vec::new();
        if dropped.type_skipped > 0 {
            parts.push(format!("{} skipped", dropped.type_skipped));
        }
        if dropped.type_unknown > 0 {
            parts.push(format!("{} skipped, type unknown", dropped.type_unknown));
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
            format::fmt_pace(max as f64)
        );
        if args.show.segments() {
            note.push_str(&format!(" ({} segments)", dropped.slow_segments));
        }
        notes.push(note);
    }
    if let Some(min) = args.min_pace {
        let mut note = format!(
            "Min pace: faster than {}/km excluded",
            format::fmt_pace(min as f64)
        );
        if args.show.segments() {
            note.push_str(&format!(" ({} segments)", dropped.fast_segments));
        }
        notes.push(note);
    }
    if args.cadence_stats && args.show.segments() {
        notes.push("Cadence columns are mean + stdev in strides/min".to_string());
    }
    if let Some(bpm) = args.hr_max.filter(|_| args.show.metrics()) {
        notes.push(format!("HR max: {bpm} bpm"));
    }
    if args.show.detail {
        notes.push(format!(
            "Passes: cadence median over {}s, passes under {}s join a neighbour",
            args.smooth, args.min_pass
        ));
    }
    notes
}

/// The CSVs of the tables that were shown, in --report-dir.
fn write_csvs(
    args: &Args,
    kept: &[&Activity],
    kept_segments: &[&Segment],
    detail: &[(&Activity, Vec<passes::Pass>)],
    metrics: Option<&metrics::Metrics>,
) -> Result<(), String> {
    let report_dir = expanduser(&args.report_dir);
    if !kept_segments.is_empty() {
        csvout::write_csvs(&report_dir, kept, kept_segments, args)?;
    }
    if !detail.is_empty() {
        csvout::write_passes(&report_dir, detail)?;
    }
    if let Some(metrics) = metrics.filter(|_| args.show.metrics()) {
        csvout::write_metrics(&report_dir, metrics, args)?;
    }
    Ok(())
}

/// routes.txt in --out, unless --route-names names another file.
fn route_names_path(args: &Args, out_dir: &Path) -> PathBuf {
    match &args.route_names {
        Some(path) => expanduser(path),
        None => out_dir.join("routes.txt"),
    }
}

/// Apply routes.txt to the routes, warning about lines that name nothing.
fn apply_route_names(
    metrics: &mut metrics::Metrics,
    args: &Args,
    out_dir: &Path,
    loaded_ids: &BTreeSet<&str>,
) -> Result<(), String> {
    let path = route_names_path(args, out_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // --name-routes and --serve create the file, so it need not exist yet.
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                && (args.route_names.is_none() || args.name_routes || args.serve) =>
        {
            return Ok(());
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let names = routes::parse_route_names(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let naming = routes::name_routes(metrics, &names);
    // An activity left out by --type, --activity or --end is not a mistake.
    for id in naming
        .unmatched
        .iter()
        .filter(|id| !loaded_ids.contains(id.as_str()))
    {
        eprintln!("warning: {}: no activity {id}", path.display());
    }
    for conflict in &naming.conflicts {
        eprintln!("warning: {}: {conflict}", path.display());
    }
    Ok(())
}

fn collect_segments(activities: &[&Activity], args: &Args) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for activity in activities {
        if !analyze::in_range(activity, args) {
            continue;
        }
        if args.method == Method::Lap {
            segments.extend(analyze::lap_segments(activity, args));
        } else {
            segments.extend(analyze::build_segments(activity, args));
        }
    }
    segments
}

fn expanduser(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut out = PathBuf::new();
    if components.peek() == Some(&std::path::Component::Normal(std::ffi::OsStr::new("~")))
        && let Ok(home) = std::env::var("HOME")
    {
        out.push(home);
        components.next();
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
