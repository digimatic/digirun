//! Command-line arguments and their defaults.

use std::path::PathBuf;

use crate::dt::Date;
use crate::report::py_num;

pub const DEFAULT_OUT: &str = "./runs";
pub const DEFAULT_REPORT_DIR: &str = "./runs/analysis";

pub const MIN_PARALLEL_FILES: usize = 8; // below this the pool costs more than it saves

pub const DEFAULT_WINDOW: i64 = 30; // seconds of track per segment
pub const DEFAULT_MIN_SEGMENT: i64 = 20; // ignore windows shorter than this
pub const DEFAULT_MIN_DISTANCE_M: f64 = 3.0; // ignore windows shorter than this
pub const DEFAULT_GAP_S: i64 = 60; // a longer pause ends the current window
pub const DEFAULT_CADENCE_THRESHOLD: i64 = 65; // strides/min; at or above this is a run
pub const DEFAULT_PACE_THRESHOLD: i64 = 540; // s/km; faster than this is a run
pub const AMBIGUOUS_CADENCE: (i64, i64) = (61, 71); // hybrid resolves this band with the pace threshold
pub const DEFAULT_TYPES: &str = "running,trail,walking";
pub const DEFAULT_MIN_PASS_S: i64 = 20; // shorter passes are absorbed into a neighbour
pub const DEFAULT_SMOOTH_S: i64 = 10; // rolling-median width for cadence in --detail
pub const DEFAULT_DETAIL_DAYS: i64 = 7; // --detail with no range covers this many days

pub const METHODS: [&str; 4] = ["hybrid", "cadence", "pace", "lap"];
pub const SHOW_TABLES: [&str; 4] = ["activities", "weekly", "overall", "all"];

/// Which report tables to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tables {
    pub activities: bool,
    pub weekly: bool,
    pub overall: bool,
}

impl Default for Tables {
    fn default() -> Tables {
        Tables {
            activities: true,
            weekly: true,
            overall: true,
        }
    }
}

const PROG: &str = "garmin-analyze";
const DESCRIPTION: &str =
    "Split downloaded runs into walk and run segments and report pace mean + stdev per class.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Hybrid,
    Cadence,
    Pace,
    Lap,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Hybrid => "hybrid",
            Method::Cadence => "cadence",
            Method::Pace => "pace",
            Method::Lap => "lap",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Args {
    pub out: PathBuf,
    pub start: Option<Date>,
    pub end: Option<Date>,
    /// Activity types to analyze; None means "all".
    pub types: Option<std::collections::BTreeSet<String>>,
    pub jobs: usize,
    pub method: Method,
    pub cadence_threshold: i64,
    pub pace_threshold: i64,
    pub window: i64,
    pub min_segment: i64,
    pub min_distance: f64,
    pub gap: i64,
    pub max_pace: Option<i64>,
    pub min_pace: Option<i64>,
    pub report_dir: PathBuf,
    pub cadence_stats: bool,
    pub no_csv: bool,
    pub show: Tables,
    /// Print the per-pass report instead of the tables.
    pub detail: bool,
    /// Activity ids to show with --detail; None means all in range.
    pub activities: Option<std::collections::BTreeSet<String>>,
    pub min_pass: i64,
    pub smooth: i64,
}

/// Worker processes/threads used to parse the files.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
}

/// The types analyzed by default; `--type all` turns the filter off.
fn default_types() -> Option<std::collections::BTreeSet<String>> {
    Some(
        DEFAULT_TYPES
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    )
}

impl Default for Args {
    fn default() -> Args {
        Args {
            out: PathBuf::from(DEFAULT_OUT),
            start: None,
            end: None,
            types: default_types(),
            jobs: default_jobs(),
            method: Method::Hybrid,
            cadence_threshold: DEFAULT_CADENCE_THRESHOLD,
            pace_threshold: DEFAULT_PACE_THRESHOLD,
            window: DEFAULT_WINDOW,
            min_segment: DEFAULT_MIN_SEGMENT,
            min_distance: DEFAULT_MIN_DISTANCE_M,
            gap: DEFAULT_GAP_S,
            max_pace: None,
            min_pace: None,
            report_dir: PathBuf::from(DEFAULT_REPORT_DIR),
            cadence_stats: false,
            no_csv: false,
            show: Tables::default(),
            detail: false,
            activities: None,
            min_pass: DEFAULT_MIN_PASS_S,
            smooth: DEFAULT_SMOOTH_S,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliError {
    pub code: i32,
    pub message: String,
    /// Help goes to stdout; errors go to stderr.
    pub stdout: bool,
}

impl CliError {
    /// A failure the program raises itself, printed without the usage block.
    fn error(message: impl Into<String>) -> CliError {
        CliError {
            code: 1,
            message: message.into(),
            stdout: false,
        }
    }

    /// A bad argument, formatted the way argparse prints it: usage, then `prog: error: ...`.
    fn argument(detail: impl AsRef<str>) -> CliError {
        CliError {
            code: 2,
            message: format!("{}\n{PROG}: error: {}", usage(), detail.as_ref()),
            stdout: false,
        }
    }
}

/// Python's repr of a string, which prefers single quotes.
fn quoted(value: &str) -> String {
    if value.contains('\'') && !value.contains('"') {
        format!("\"{value}\"")
    } else {
        format!("'{value}'")
    }
}

/// Accept `9:00`, `9:30` or plain seconds.
pub fn parse_pace(value: &str, name: &str) -> Result<i64, CliError> {
    if value.contains(':') {
        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() >= 2 {
            if let (Ok(minutes), Ok(seconds)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
                return Ok(minutes * 60 + seconds);
            }
        }
    } else if let Ok(seconds) = value.parse::<f64>() {
        return Ok(seconds as i64);
    }
    Err(CliError::error(format!(
        "error: invalid {name} {}, expected M:SS",
        quoted(value)
    )))
}

/// Accept `YYYY-MM-DD`.
pub fn parse_date(value: &str, name: &str) -> Result<Date, CliError> {
    match Date::from_iso(value) {
        Some(date) => Ok(date),
        None => Err(CliError::error(format!(
            "error: invalid {name} {}, expected YYYY-MM-DD",
            quoted(value)
        ))),
    }
}

struct Action {
    /// Option strings, e.g. `["-h", "--help"]`. The usage line uses the first one.
    strings: Vec<String>,
    /// Empty for flags that take no value.
    metavar: String,
    help: String,
}

impl Action {
    fn new(strings: &[&str], metavar: &str, help: String) -> Action {
        Action {
            strings: strings.iter().map(|s| s.to_string()).collect(),
            metavar: metavar.to_string(),
            help,
        }
    }

    /// How the action is listed in the help body: every option string, then the metavar.
    fn invocation(&self) -> String {
        let options = self.strings.join(", ");
        if self.metavar.is_empty() {
            options
        } else {
            format!("{options} {metavar}", metavar = self.metavar)
        }
    }

    /// How the action appears in the usage line: the first option string only.
    fn usage_part(&self) -> String {
        let first = self.strings.first().map(|s| s.as_str()).unwrap_or_default();
        if self.metavar.is_empty() {
            format!("[{first}]")
        } else {
            format!("[{first} {metavar}]", metavar = self.metavar)
        }
    }
}

struct Group {
    title: String,
    actions: Vec<Action>,
}

/// The help groups, in the order argparse prints them.
fn groups() -> Vec<Group> {
    let jobs = default_jobs();
    // argparse joins choices in the metavar without spaces.
    let method_choices = format!("{{{}}}", METHODS.join(","));
    vec![
        Group {
            title: "options".to_string(),
            actions: vec![Action::new(
                &["-h", "--help"],
                "",
                "show this help message and exit".to_string(),
            )],
        },
        Group {
            title: "input".to_string(),
            actions: vec![
                Action::new(
                    &["--out"],
                    "OUT",
                    format!("directory of downloaded files (default: {DEFAULT_OUT})"),
                ),
                Action::new(
                    &["--start"],
                    "YYYY-MM-DD",
                    "only activities from this date".to_string(),
                ),
                Action::new(
                    &["--end"],
                    "YYYY-MM-DD",
                    "only activities up to this date".to_string(),
                ),
                Action::new(
                    &["--type"],
                    "TYPE",
                    format!(
                        "activity types to analyze: 'all' or comma-separated keys (default: {DEFAULT_TYPES}); types come from summary.csv"
                    ),
                ),
                Action::new(
                    &["--jobs"],
                    "N",
                    format!(
                        "worker processes used to parse the files (default: {jobs} = min(cpu_count, 8); 1 runs single-process)"
                    ),
                ),
            ],
        },
        Group {
            title: "classification".to_string(),
            actions: vec![
                Action::new(
                    &["--method"],
                    &method_choices,
                    "how to tell walk from run (default: hybrid)".to_string(),
                ),
                Action::new(
                    &["--cadence-threshold"],
                    "CADENCE_THRESHOLD",
                    format!(
                        "strides/min at or above which a segment is a run (default: {DEFAULT_CADENCE_THRESHOLD})"
                    ),
                ),
                Action::new(
                    &["--pace-threshold"],
                    "PACE_THRESHOLD",
                    format!(
                        "pace cutoff as M:SS or seconds (default: {DEFAULT_PACE_THRESHOLD}s = 9:00/km)"
                    ),
                ),
            ],
        },
        Group {
            title: "segmentation".to_string(),
            actions: vec![
                Action::new(
                    &["--window"],
                    "WINDOW",
                    format!("seconds of track per segment (default: {DEFAULT_WINDOW})"),
                ),
                Action::new(
                    &["--min-segment"],
                    "MIN_SEGMENT",
                    format!(
                        "ignore segments shorter than this many seconds (default: {DEFAULT_MIN_SEGMENT})"
                    ),
                ),
                Action::new(
                    &["--min-distance"],
                    "MIN_DISTANCE",
                    format!(
                        "ignore segments shorter than this many metres (default: {})",
                        py_num(DEFAULT_MIN_DISTANCE_M)
                    ),
                ),
                Action::new(
                    &["--gap"],
                    "GAP",
                    format!("a pause longer than this ends the segment (default: {DEFAULT_GAP_S}s)"),
                ),
                Action::new(
                    &["--max-pace"],
                    "M:SS",
                    "exclude segments slower than this from the stats, e.g. 15:00 (useful for filtering out standing-about segments)".to_string(),
                ),
                Action::new(
                    &["--min-pace"],
                    "M:SS",
                    "exclude segments faster than this from the stats, e.g. 3:00 (drops GPS-glitch segments no human could cover)".to_string(),
                ),
            ],
        },
        Group {
            title: "detail".to_string(),
            actions: vec![
                Action::new(
                    &["--detail"],
                    "",
                    format!(
                        "list every run/walk pass, split on cadence, with its stats; the range is --start/--end, or --activity, or by default the last {DEFAULT_DETAIL_DAYS} days (e.g. --detail --start 2026-10-01)"
                    ),
                ),
                Action::new(
                    &["--activity"],
                    "ID",
                    "only these comma-separated activity ids".to_string(),
                ),
                Action::new(
                    &["--min-pass"],
                    "MIN_PASS",
                    format!(
                        "passes shorter than this many seconds join a neighbour (default: {DEFAULT_MIN_PASS_S})"
                    ),
                ),
                Action::new(
                    &["--smooth"],
                    "SMOOTH",
                    format!(
                        "seconds of rolling median applied to cadence (default: {DEFAULT_SMOOTH_S})"
                    ),
                ),
            ],
        },
        Group {
            title: "output".to_string(),
            actions: vec![
                Action::new(
                    &["--report-dir"],
                    "REPORT_DIR",
                    format!(
                        "where analysis.csv and segments.csv (passes.csv with --detail) are written (default: {DEFAULT_REPORT_DIR})"
                    ),
                ),
                Action::new(
                    &["--cadence-stats"],
                    "",
                    "also report cadence mean + stdev in strides/min".to_string(),
                ),
                Action::new(
                    &["--show"],
                    "TABLES",
                    "report tables to print: 'all' or comma-separated activities,weekly,overall (default: all)".to_string(),
                ),
                Action::new(&["--no-csv"], "", "print the report only".to_string()),
            ],
        },
    ]
}

/// argparse formats at the terminal width, which falls back to 80 columns when
/// the output is not a terminal. COLUMNS is honoured the same way.
fn format_width() -> usize {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(80);
    columns.saturating_sub(2)
}

/// Greedy word wrapping, as textwrap.wrap does it.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub fn usage() -> String {
    let action_groups = groups();
    let parts: Vec<String> = action_groups
        .iter()
        .flat_map(|group| group.actions.iter())
        .map(|action| action.usage_part())
        .collect();
    let width = format_width();
    let prefix = "usage: ";
    let single = format!("{prefix}{PROG} {}", parts.join(" "));
    if single.chars().count() <= width {
        return single;
    }

    // Too long for one line: put the parts on continuation lines, as argparse does.
    let indent = " ".repeat(prefix.chars().count() + PROG.chars().count() + 1);
    let mut lines = Vec::new();
    let mut current: Vec<String> = Vec::new();
    // The first line starts with the prefix instead of the indent.
    let mut length = prefix.chars().count() - 1;
    for part in std::iter::once(PROG.to_string()).chain(parts) {
        if !current.is_empty() && length + 1 + part.chars().count() > width {
            lines.push(format!("{indent}{}", current.join(" ")));
            current.clear();
            length = indent.chars().count() - 1;
        }
        length += 1 + part.chars().count();
        current.push(part);
    }
    if !current.is_empty() {
        lines.push(format!("{indent}{}", current.join(" ")));
    }

    let mut out = String::from(prefix);
    out.push_str(&lines[0][indent.len()..]);
    for line in &lines[1..] {
        out.push('\n');
        out.push_str(line);
    }
    out
}

pub fn help() -> String {
    let width = format_width();
    let action_groups = groups();
    let max_length = action_groups
        .iter()
        .flat_map(|group| group.actions.iter())
        .map(|action| action.invocation().chars().count() + 2)
        .max()
        .unwrap_or(0);
    let help_position = (max_length + 2).min(24);
    let action_width = help_position - 4;
    let help_width = width.saturating_sub(help_position).max(11);

    let mut lines = vec![usage(), String::new()];
    lines.extend(wrap(DESCRIPTION, width));
    lines.push(String::new());

    for (index, group) in action_groups.iter().enumerate() {
        lines.push(format!("{}:", group.title));
        for action in &group.actions {
            let invocation = action.invocation();
            let help_lines = wrap(&action.help, help_width);
            if help_lines.is_empty() {
                lines.push(format!("  {invocation}"));
                continue;
            }
            if invocation.chars().count() <= action_width {
                let padding = " ".repeat(action_width - invocation.chars().count());
                lines.push(format!("  {invocation}{padding}  {}", help_lines[0]));
            } else {
                lines.push(format!("  {invocation}"));
                lines.push(format!("{}{}", " ".repeat(help_position), help_lines[0]));
            }
            for line in &help_lines[1..] {
                lines.push(format!("{}{}", " ".repeat(help_position), line));
            }
        }
        if index + 1 < action_groups.len() {
            lines.push(String::new());
        }
    }

    lines.join("\n")
}

/// Parse argv (without the program name).
pub fn parse(argv: &[String]) -> Result<Args, CliError> {
    let mut args = Args::default();
    let mut i = 0;

    while i < argv.len() {
        let (flag, inline) = match argv[i].split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (argv[i].as_str(), None),
        };

        // Flags that take a value: use the inline value when given, otherwise the next argv.
        let value = |i: &mut usize| -> Result<String, CliError> {
            if let Some(v) = inline {
                return Ok(v);
            }
            if *i + 1 >= argv.len() {
                return Err(CliError::argument(format!(
                    "argument {flag}: expected one argument"
                )));
            }
            *i += 1;
            Ok(argv[*i].clone())
        };

        match flag {
            "-h" | "--help" => {
                return Err(CliError {
                    code: 0,
                    message: help(),
                    stdout: true,
                })
            }
            "--out" => args.out = PathBuf::from(value(&mut i)?),
            "--start" => args.start = Some(parse_date(&value(&mut i)?, "start date")?),
            "--end" => args.end = Some(parse_date(&value(&mut i)?, "end date")?),
            "--type" => {
                let raw = value(&mut i)?;
                if raw == "all" {
                    args.types = None;
                } else {
                    let set: std::collections::BTreeSet<String> = raw
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                    args.types = if set.is_empty() { None } else { Some(set) };
                }
            }
            "--jobs" => {
                let raw = value(&mut i)?;
                args.jobs = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --jobs: invalid int value: {}",
                        quoted(&raw)
                    ))
                })? as usize;
            }
            "--method" => {
                let raw = value(&mut i)?;
                args.method = match raw.as_str() {
                    "hybrid" => Method::Hybrid,
                    "cadence" => Method::Cadence,
                    "pace" => Method::Pace,
                    "lap" => Method::Lap,
                    _ => {
                        return Err(CliError::argument(format!(
                            "argument --method: invalid choice: {} (choose from {})",
                            quoted(&raw),
                            METHODS.join(", ")
                        )))
                    }
                };
            }
            "--cadence-threshold" => {
                let raw = value(&mut i)?;
                args.cadence_threshold = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --cadence-threshold: invalid int value: {}",
                        quoted(&raw)
                    ))
                })?;
            }
            "--pace-threshold" => {
                args.pace_threshold = parse_pace(&value(&mut i)?, "pace")?;
            }
            "--window" => {
                let raw = value(&mut i)?;
                args.window = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --window: invalid int value: {}",
                        quoted(&raw)
                    ))
                })?;
            }
            "--min-segment" => {
                let raw = value(&mut i)?;
                args.min_segment = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --min-segment: invalid int value: {}",
                        quoted(&raw)
                    ))
                })?;
            }
            "--min-distance" => {
                let raw = value(&mut i)?;
                args.min_distance = raw.parse::<f64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --min-distance: invalid float value: {}",
                        quoted(&raw)
                    ))
                })?;
                if !args.min_distance.is_finite() {
                    return Err(CliError::argument(format!(
                        "argument --min-distance: invalid float value: {}",
                        quoted(&raw)
                    )));
                }
            }
            "--gap" => {
                let raw = value(&mut i)?;
                args.gap = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --gap: invalid int value: {}",
                        quoted(&raw)
                    ))
                })?;
            }
            // Python treats a pace guard of 0 as "not set", so the same is done here.
            "--max-pace" => {
                args.max_pace = Some(parse_pace(&value(&mut i)?, "max pace")?).filter(|v| *v != 0)
            }
            "--min-pace" => {
                args.min_pace = Some(parse_pace(&value(&mut i)?, "min pace")?).filter(|v| *v != 0)
            }
            "--report-dir" => args.report_dir = PathBuf::from(value(&mut i)?),
            "--cadence-stats" => args.cadence_stats = true,
            "--no-csv" => args.no_csv = true,
            "--show" => {
                let raw = value(&mut i)?;
                let mut show = Tables {
                    activities: false,
                    weekly: false,
                    overall: false,
                };
                for name in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                    match name {
                        "activities" => show.activities = true,
                        "weekly" => show.weekly = true,
                        "overall" => show.overall = true,
                        "all" => show = Tables::default(),
                        _ => {
                            return Err(CliError::argument(format!(
                                "argument --show: invalid choice: {} (choose from {})",
                                quoted(name),
                                SHOW_TABLES.join(", ")
                            )))
                        }
                    }
                }
                args.show = show;
            }
            "--detail" => args.detail = true,
            "--activity" => {
                let set: std::collections::BTreeSet<String> = value(&mut i)?
                    .split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
                args.activities = if set.is_empty() { None } else { Some(set) };
            }
            "--min-pass" => {
                let raw = value(&mut i)?;
                args.min_pass = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --min-pass: invalid int value: {}",
                        quoted(&raw)
                    ))
                })?;
            }
            "--smooth" => {
                let raw = value(&mut i)?;
                args.smooth = raw.parse::<i64>().map_err(|_| {
                    CliError::argument(format!(
                        "argument --smooth: invalid int value: {}",
                        quoted(&raw)
                    ))
                })?;
            }
            other => {
                return Err(CliError::argument(format!(
                    "unrecognized arguments: {other}"
                )))
            }
        }
        i += 1;
    }

    args.jobs = args.jobs.max(1);
    Ok(args)
}
