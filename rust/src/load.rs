//! Reading the downloaded files into activities.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::args::MIN_PARALLEL_FILES;
use crate::csv;
use crate::dt::Date;
use crate::model::Activity;
use crate::parse::{read_gpx, read_tcx};
use crate::xml;

/// One activity group: the GPX and TCX files that share an activity id.
pub type Files = BTreeMap<String, PathBuf>;

pub struct Loaded {
    pub activities: Vec<Activity>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

/// Activity id from the download filename (trailing `_<id>`), else the stem.
pub fn activity_id(path: &Path) -> String {
    let stem = stem(path);
    match stem.rsplit_once('_') {
        Some((_, id)) if is_digits(id) => id.to_string(),
        _ => stem,
    }
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Activity name from the download filename, without the timestamp prefix.
pub fn name_from_filename(path: &Path) -> String {
    let stem = stem(path);
    let parts: Vec<&str> = stem.split('_').collect();
    let name = if parts.len() >= 3 {
        parts[2..].join("_")
    } else {
        stem.clone()
    };
    match name.rsplit_once('_') {
        Some((head, _)) if !head.is_empty() => head.to_string(),
        _ => {
            if name.is_empty() {
                stem
            } else {
                name
            }
        }
    }
}

pub fn date_from_filename(path: &Path) -> Option<Date> {
    let stem = stem(path);
    Date::from_iso(&stem.chars().take(10).collect::<String>())
}

/// activity_id -> type key, from the summary.csv written by garmin-runs.
///
/// The GPX/TCX files themselves do not record the activity type, so summary.csv
/// is the only source. Returns an empty map when it is missing.
pub fn activity_types(out_dir: &Path) -> BTreeMap<String, String> {
    let content = match std::fs::read_to_string(out_dir.join("summary.csv")) {
        Ok(content) => content,
        Err(_) => return BTreeMap::new(),
    };
    let records = csv::read_records(&content);
    let id_index = match csv::column(&records, "activity_id") {
        Some(index) => index,
        None => return BTreeMap::new(),
    };
    let type_index = csv::column(&records, "type");

    let mut types = BTreeMap::new();
    for row in records.iter().skip(1) {
        let id = match row.get(id_index) {
            Some(id) => id.clone(),
            None => continue,
        };
        types.insert(
            id,
            type_index
                .and_then(|i| row.get(i))
                .cloned()
                .unwrap_or_default(),
        );
    }
    types
}

/// Read one activity group into an Activity, or None when it has no points.
pub fn read_group(aid: &str, files: &Files) -> Result<Option<Activity>, String> {
    let gpx = files.get(".gpx");
    let tcx = files.get(".tcx");

    let (gpx_points, gpx_name) = match gpx {
        Some(path) => {
            let root = parse_file(path)?;
            read_gpx(&root)
        }
        None => (Vec::new(), String::new()),
    };
    let (tcx_points, tcx_laps) = match tcx {
        Some(path) => {
            let root = parse_file(path)?;
            read_tcx(&root)
        }
        None => (Vec::new(), Vec::new()),
    };

    // TCX is preferred: it carries Garmin's own distances, laps and speed.
    let points = if tcx_points.is_empty() {
        gpx_points
    } else {
        tcx_points
    };
    if points.is_empty() {
        return Ok(None);
    }

    // The name and date come from the GPX file when there is one, as in Python.
    let source = gpx.or(tcx).unwrap();
    let name = if gpx_name.is_empty() {
        name_from_filename(source)
    } else {
        gpx_name
    };
    let when = date_from_filename(source).unwrap_or(points[0].time.date);

    Ok(Some(Activity {
        activity_id: aid.to_string(),
        name,
        date: when,
        points,
        laps: tcx_laps,
        r#type: String::new(),
    }))
}

fn parse_file(path: &Path) -> Result<xml::Node, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    xml::parse(&content).map_err(|e| format!("{}: {e}", path.display()))
}

/// Group the downloaded files by activity id and read them into activities.
///
/// Groups are built in sorted order and results are collected in that same
/// order, so the result is identical with or without the pool.
/// Group the downloaded files by activity id, in sorted order.
pub fn groups(out_dir: &Path) -> Vec<(String, Files)> {
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(out_dir) {
        Ok(read_dir) => read_dir
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort();

    let mut groups: BTreeMap<String, Files> = BTreeMap::new();
    for path in entries {
        let suffix = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if suffix != "gpx" && suffix != "tcx" {
            continue;
        }
        groups
            .entry(activity_id(&path))
            .or_default()
            .insert(format!(".{suffix}"), path);
    }

    groups
        .into_iter()
        .map(|(aid, files)| (aid, files))
        .collect()
}

pub fn load_activities(out_dir: &Path, jobs: usize) -> Loaded {
    let work: Arc<Vec<(String, Files)>> = Arc::new(groups(out_dir));

    let results: Arc<Mutex<Vec<Option<Result<Option<Activity>, String>>>>> =
        Arc::new(Mutex::new(vec![None; work.len()]));
    let next = Arc::new(AtomicUsize::new(0));

    if jobs > 1 && work.len() >= MIN_PARALLEL_FILES {
        let handles: Vec<_> = (0..jobs.min(work.len()))
            .map(|_| {
                let work = Arc::clone(&work);
                let results = Arc::clone(&results);
                let next = Arc::clone(&next);
                std::thread::spawn(move || loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= work.len() {
                        break;
                    }
                    let result = read_group(&work[index].0, &work[index].1);
                    results.lock().unwrap()[index] = Some(result);
                })
            })
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
    } else {
        let mut guard = results.lock().unwrap();
        for (index, (aid, files)) in work.iter().enumerate() {
            guard[index] = Some(read_group(aid, files));
        }
    }

    let collected = results.lock().unwrap().clone();
    let mut activities: Vec<Activity> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for ((aid, _), result) in work.iter().zip(collected) {
        match result {
            Some(Ok(Some(activity))) => activities.push(activity),
            Some(Ok(None)) => skipped.push(aid.clone()),
            Some(Err(error)) => errors.push(error),
            None => {}
        }
    }

    activities
        .sort_by(|a, b| (a.date, a.activity_id.as_str()).cmp(&(b.date, b.activity_id.as_str())));
    Loaded {
        activities,
        skipped,
        errors,
    }
}
