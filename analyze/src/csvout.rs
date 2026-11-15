//! Writing `analysis.csv` and `segments.csv`.

use std::collections::BTreeMap;
use std::path::Path;

use crate::args::Args;
use crate::csv::Writer;
use crate::format::py_num;
use crate::metrics::{Metrics, sessions_in_range};
use crate::model::{Activity, Segment};
use crate::num::round;
use crate::passes::Pass;
use crate::segments::stats;

pub const ANALYSIS_FIELDS: [&str; 13] = [
    "date",
    "activity_id",
    "name",
    "type",
    "class",
    "segments",
    "time_s",
    "distance_km",
    "mean_pace_s_per_km",
    "stdev_pace_s_per_km",
    "min_pace_s_per_km",
    "max_pace_s_per_km",
    "share_pct",
];

/// Added to analysis.csv only with --cadence-stats.
pub const CADENCE_ANALYSIS_FIELDS: [&str; 2] = ["mean_cadence", "stdev_cadence"];

pub const SEGMENT_FIELDS: [&str; 8] = [
    "activity_id",
    "date",
    "time",
    "class",
    "pace_s_per_km",
    "cadence",
    "distance_m",
    "window_s",
];

pub fn write_csvs(
    report_dir: &Path,
    activities: &[&Activity],
    segments: &[&Segment],
    args: &Args,
) -> Result<(), String> {
    std::fs::create_dir_all(report_dir).map_err(|e| format!("{}: {e}", report_dir.display()))?;

    let mut by_activity: BTreeMap<&str, Vec<&Segment>> = BTreeMap::new();
    for s in segments {
        by_activity
            .entry(s.activity_id.as_str())
            .or_default()
            .push(s);
    }

    let mut fields: Vec<&str> = ANALYSIS_FIELDS.to_vec();
    if args.cadence_stats {
        fields.extend(CADENCE_ANALYSIS_FIELDS);
    }

    let mut writer = Writer::new();
    writer.row(&fields.iter().map(|f| f.to_string()).collect::<Vec<_>>());
    for activity in activities {
        let rows = match by_activity.get(activity.activity_id.as_str()) {
            Some(rows) => rows,
            None => continue,
        };
        if rows.is_empty() {
            continue;
        }
        let total_time: f64 = rows.iter().map(|s| s.duration_s).sum();
        for class in ["run", "walk"] {
            let group: Vec<&Segment> = rows.iter().filter(|s| s.class == class).copied().collect();
            if group.is_empty() {
                continue;
            }
            let st = stats(&group);
            let share = if total_time > 0.0 {
                py_num(round(st.time_s / total_time * 100.0, 1))
            } else {
                String::new()
            };
            let mut row = vec![
                activity.date.isoformat(),
                activity.activity_id.clone(),
                activity.name.clone(),
                activity.r#type.clone(),
                class.to_string(),
                st.segments.to_string(),
                py_num(st.time_s),
                py_num(st.distance_km),
                py_num(st.mean),
                py_num(st.stdev),
                py_num(st.min),
                py_num(st.max),
                share,
            ];
            if args.cadence_stats {
                row.push(st.cad_mean.map(py_num).unwrap_or_default());
                row.push(st.cad_stdev.map(py_num).unwrap_or_default());
            }
            writer.row(&row);
        }
    }

    let analysis_path = report_dir.join("analysis.csv");
    write_file(&analysis_path, writer.contents())?;

    let mut writer = Writer::new();
    writer.row(
        &SEGMENT_FIELDS
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>(),
    );
    let mut ordered: Vec<&Segment> = segments.to_vec();
    ordered.sort_by_key(|a| (a.date, a.time.instant()));
    for s in ordered {
        writer.row(&[
            s.activity_id.clone(),
            s.date.isoformat(),
            s.time.isoformat(),
            s.class.to_string(),
            py_num(round(s.pace_s_per_km, 1)),
            s.cadence
                .map(|cadence| py_num(round(cadence, 1)))
                .unwrap_or_default(),
            py_num(round(s.distance_m, 1)),
            py_num(round(s.duration_s, 1)),
        ]);
    }

    let segment_path = report_dir.join("segments.csv");
    write_file(&segment_path, writer.contents())?;

    println!();
    println!(
        "Wrote {} and {}",
        display(&analysis_path),
        display(&segment_path)
    );
    Ok(())
}

pub const PASS_FIELDS: [&str; 30] = [
    "activity_id",
    "date",
    "name",
    "type",
    "pass",
    "class",
    "source",
    "excluded",
    "start_time",
    "start_offset_s",
    "time_s",
    "stopped_s",
    "distance_m",
    "pace_s_per_km",
    "stdev_pace_s_per_km",
    "min_pace_s_per_km",
    "max_pace_s_per_km",
    "mean_cadence",
    "stdev_cadence",
    "min_cadence",
    "max_cadence",
    "mean_hr",
    "min_hr",
    "max_hr",
    "hr_drift",
    "elevation_gain_m",
    "elevation_loss_m",
    "stride_m",
    "laps",
    "end_time",
];

/// Write `passes.csv`, one row per pass.
pub fn write_passes(
    report_dir: &Path,
    activities: &[(&Activity, Vec<Pass>)],
) -> Result<(), String> {
    std::fs::create_dir_all(report_dir).map_err(|e| format!("{}: {e}", report_dir.display()))?;
    let num = |value: Option<f64>| value.map(py_num).unwrap_or_default();

    let mut writer = Writer::new();
    writer.row(
        &PASS_FIELDS
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>(),
    );
    for (activity, passes) in activities {
        for (i, p) in passes.iter().enumerate() {
            let s = &p.stats;
            writer.row(&[
                activity.activity_id.clone(),
                activity.date.isoformat(),
                activity.name.clone(),
                activity.r#type.clone(),
                (i + 1).to_string(),
                p.class.to_string(),
                p.source.as_str().to_string(),
                (p.excluded as u8).to_string(),
                activity.points[p.start].time.isoformat(),
                py_num(round(s.start_offset_s, 1)),
                py_num(s.time_s),
                py_num(s.stopped_s),
                py_num(s.distance_m),
                num(s.pace.filter(|_| s.pace_reliable)),
                num(s.pace_sd),
                num(s.pace_min),
                num(s.pace_max),
                num(s.cad_mean),
                num(s.cad_sd),
                num(s.cad_min),
                num(s.cad_max),
                num(s.hr_mean),
                num(s.hr_min),
                num(s.hr_max),
                num(s.hr_drift),
                num(s.elev_gain),
                num(s.elev_loss),
                num(s.stride_m),
                p.laps
                    .iter()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                activity.points[p.end].time.isoformat(),
            ]);
        }
    }

    let path = report_dir.join("passes.csv");
    write_file(&path, writer.contents())?;
    println!();
    println!("Wrote {}", display(&path));
    Ok(())
}

/// Path as Python's pathlib prints it: a leading `./` is dropped.
fn display(path: &Path) -> String {
    let value = path.to_string_lossy();
    match value.strip_prefix("./") {
        Some(stripped) => stripped.to_string(),
        None => value.into_owned(),
    }
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

pub const SESSION_FIELDS: [&str; 22] = [
    "date",
    "activity_id",
    "name",
    "type",
    "route",
    "route_name",
    "run_s",
    "walk_s",
    "run_m",
    "walk_m",
    "longest_run_s",
    "longest_run_m",
    "run_pace_s_per_km",
    "grade_adjusted_pace_s_per_km",
    "run_hr",
    "efficiency",
    "decoupling_pct",
    "zone3_pace_s_per_km",
    "hr_recovery_60s",
    "elevation_gain_m",
    "trimp",
    "km",
];

pub const WEEK_FIELDS: [&str; 20] = [
    "week",
    "monday",
    "sessions",
    "longest_run_s",
    "longest_run_m",
    "run_s",
    "walk_s",
    "km",
    "km_previous_4_weeks",
    "trimp",
    "run_zone1_s",
    "run_zone2_s",
    "run_zone3_s",
    "run_zone4_s",
    "run_zone5_s",
    "all_zone1_s",
    "all_zone2_s",
    "all_zone3_s",
    "all_zone4_s",
    "all_zone5_s",
];

/// Write `sessions.csv` and `weeks.csv`, narrowed to --start/--end.
pub fn write_metrics(report_dir: &Path, metrics: &Metrics, args: &Args) -> Result<(), String> {
    std::fs::create_dir_all(report_dir).map_err(|e| format!("{}: {e}", report_dir.display()))?;
    let num = |value: Option<f64>, digits: usize| {
        value.map(|v| py_num(round(v, digits))).unwrap_or_default()
    };
    let fields = |names: &[&str]| names.iter().map(|f| f.to_string()).collect::<Vec<_>>();

    let mut writer = Writer::new();
    writer.row(&fields(&SESSION_FIELDS));
    for s in sessions_in_range(metrics, args) {
        writer.row(&[
            s.activity.date.isoformat(),
            s.activity.activity_id.clone(),
            s.activity.name.clone(),
            s.activity.r#type.clone(),
            s.route.map(|r| format!("R{}", r + 1)).unwrap_or_default(),
            s.route.map_or(String::new(), |_| s.route_name.clone()),
            py_num(round(s.run_s, 1)),
            py_num(round(s.walk_s, 1)),
            py_num(round(s.run_m, 1)),
            py_num(round(s.walk_m, 1)),
            py_num(round(s.longest_run_s, 1)),
            py_num(round(s.longest_run_m, 1)),
            num(s.run_pace(), 1),
            num(s.grade_adjusted_pace, 1),
            num(s.run_hr, 1),
            num(s.efficiency, 3),
            num(s.decoupling, 1),
            num(s.zone3_pace, 1),
            num(s.recovery, 1),
            num(s.elev_gain, 1),
            num(s.trimp, 1),
            py_num(round(s.km(), 2)),
        ]);
    }
    let sessions_path = report_dir.join("sessions.csv");
    write_file(&sessions_path, writer.contents())?;

    let mut writer = Writer::new();
    writer.row(&fields(&WEEK_FIELDS));
    for week in metrics.weeks.iter().filter(|w| w.in_range(args)) {
        let mut row = vec![
            week.label(),
            week.monday.isoformat(),
            week.sessions.to_string(),
            py_num(round(week.longest_run_s, 1)),
            py_num(round(week.longest_run_m, 1)),
            py_num(round(week.run_s, 1)),
            py_num(round(week.walk_s, 1)),
            py_num(round(week.km, 2)),
            num(week.km_baseline, 2),
            num(week.trimp, 1),
        ];
        for zones in [&week.zones_run, &week.zones_all] {
            row.extend(zones.iter().map(|s| py_num(round(*s, 1))));
        }
        writer.row(&row);
    }
    let weeks_path = report_dir.join("weeks.csv");
    write_file(&weeks_path, writer.contents())?;

    println!();
    println!(
        "Wrote {} and {}",
        display(&sessions_path),
        display(&weeks_path)
    );
    Ok(())
}
