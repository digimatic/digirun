//! Writing `analysis.csv` and `segments.csv`.

use std::collections::BTreeMap;
use std::path::Path;

use crate::analyze::{round, stats};
use crate::args::Args;
use crate::csv::Writer;
use crate::model::{Activity, Segment};
use crate::passes::Pass;
use crate::report::py_num;

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
    ordered.sort_by(|a, b| (a.date, a.time.instant()).cmp(&(b.date, b.time.instant())));
    for s in ordered {
        writer.row(&vec![
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

pub const PASS_FIELDS: [&str; 29] = [
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
                py_num(s.distance_m),
                num(s.pace),
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
