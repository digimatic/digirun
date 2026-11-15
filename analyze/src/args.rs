//! Command-line arguments and their defaults.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, ValueEnum};

use crate::dt::Date;

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
pub const DEFAULT_TYPES: &str = "running,trail";
pub const DEFAULT_MIN_PASS_S: i64 = 20; // shorter passes are absorbed into a neighbour
pub const DEFAULT_PORT: u16 = 8765; // --serve listens here, on 127.0.0.1 only
pub const DEFAULT_NAMING_DAYS: i64 = 30; // --name-routes asks about this many days
pub const DEFAULT_SMOOTH_S: i64 = 10; // rolling-median width for cadence in the detail table

/// Which report tables to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tables {
    pub activities: bool,
    pub weekly: bool,
    pub overall: bool,
    /// Every run/walk pass, split on cadence (src/passes.rs).
    pub detail: bool,
    /// This week against the previous four (src/metrics.rs, like the rest below).
    pub summary: bool,
    /// Weekly volume and the longest continuous run.
    pub progress: bool,
    /// Weekly time in HR zones, TRIMP and acute:chronic load.
    pub zones: bool,
    /// Efficiency, decoupling, HR recovery and grade-adjusted pace, per route.
    pub fitness: bool,
    /// Best efforts and race predictions.
    pub efforts: bool,
}

impl Tables {
    pub fn none() -> Tables {
        Tables {
            activities: false,
            weekly: false,
            overall: false,
            detail: false,
            summary: false,
            progress: false,
            zones: false,
            fitness: false,
            efforts: false,
        }
    }

    pub fn all() -> Tables {
        Tables {
            activities: true,
            weekly: true,
            overall: true,
            detail: true,
            summary: true,
            progress: true,
            zones: true,
            fitness: true,
            efforts: true,
        }
    }

    /// Any of the tables built from the --window segments.
    pub fn segments(&self) -> bool {
        self.activities || self.weekly || self.overall
    }

    /// Any of the tables built from the session metrics.
    pub fn metrics(&self) -> bool {
        self.summary || self.progress || self.zones || self.fitness || self.efforts
    }

    /// Parse `--show`: 'all' or comma-separated table names.
    fn parse(raw: &str) -> Result<Tables, String> {
        let mut show = Tables::none();
        for name in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            match name {
                "activities" => show.activities = true,
                "weekly" => show.weekly = true,
                "overall" => show.overall = true,
                "detail" => show.detail = true,
                "summary" => show.summary = true,
                "progress" => show.progress = true,
                "zones" => show.zones = true,
                "fitness" => show.fitness = true,
                "efforts" => show.efforts = true,
                "all" => show = Tables::all(),
                _ => {
                    return Err(format!(
                        "invalid choice '{name}' (choose from activities, weekly, overall, \
                         detail, summary, progress, zones, fitness, efforts, all)"
                    ));
                }
            }
        }
        Ok(show)
    }
}

impl Default for Tables {
    /// The segment tables; the others are long or answer a narrower question.
    fn default() -> Tables {
        Tables {
            activities: true,
            weekly: true,
            overall: true,
            ..Tables::none()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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
    pub types: Option<BTreeSet<String>>,
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
    /// `--days N`, already resolved into `start`; kept for the report header.
    pub days: Option<i64>,
    /// Activity ids to analyze; None means all in range.
    pub activities: Option<BTreeSet<String>>,
    pub min_pass: i64,
    pub smooth: i64,
    /// Maximum heart rate; HR zones and TRIMP need it, and it is never guessed.
    pub hr_max: Option<i64>,
    /// Where to write the HTML report, if anywhere.
    pub html: Option<PathBuf>,
    /// Add satellite maps to the HTML report, which then holds GPS positions.
    pub map: bool,
    /// Route names file; None means routes.txt in --out, if it exists.
    pub route_names: Option<PathBuf>,
    /// Ask for the names of the routes run in the range and write them.
    pub name_routes: bool,
    /// Serve the report on localhost, with route cards that can be renamed.
    pub serve: bool,
    pub port: u16,
}

/// Worker threads used to parse the files.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(8)
}

impl Default for Args {
    fn default() -> Args {
        Cli::parse_from(["garmin-analyze"]).into_args()
    }
}

#[derive(Debug, Clone)]
pub struct CliError {
    pub code: i32,
    pub message: String,
    /// Help goes to stdout; errors go to stderr.
    pub stdout: bool,
}

impl From<clap::Error> for CliError {
    fn from(error: clap::Error) -> CliError {
        CliError {
            code: error.exit_code(),
            message: error.render().to_string().trim_end().to_string(),
            stdout: !error.use_stderr(),
        }
    }
}

/// Accept `9:00`, `9:30` or plain seconds.
pub fn parse_pace(value: &str) -> Result<i64, String> {
    if let Some((minutes, seconds)) = value.split_once(':') {
        if let (Ok(minutes), Ok(seconds)) = (minutes.parse::<i64>(), seconds.parse::<i64>()) {
            return Ok(minutes * 60 + seconds);
        }
    } else if let Ok(seconds) = value.parse::<f64>() {
        return Ok(seconds as i64);
    }
    Err("expected M:SS or seconds".to_string())
}

/// Accept `YYYY-MM-DD`.
pub fn parse_date(value: &str) -> Result<Date, String> {
    Date::from_iso(value).ok_or_else(|| "expected YYYY-MM-DD".to_string())
}

fn parse_distance(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| "expected a number of metres".to_string())
}

/// A comma-separated option value as a set, or None when it names nothing.
fn list(raw: &str) -> Option<BTreeSet<String>> {
    let set: BTreeSet<String> = raw
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    (!set.is_empty()).then_some(set)
}

/// Split downloaded runs into walk and run segments and report pace mean + stdev per class.
#[derive(Debug, Parser)]
#[command(name = "garmin-analyze", version)]
struct Cli {
    /// Directory of downloaded files
    #[arg(long, value_name = "OUT", default_value = DEFAULT_OUT, help_heading = "Input")]
    out: PathBuf,
    /// Route names, one '<activity id> = <name>' per line; any session of a route names it [default: routes.txt in --out]
    #[arg(long, value_name = "PATH", help_heading = "Input")]
    route_names: Option<PathBuf>,
    /// Only activities from this date
    #[arg(long, value_name = "YYYY-MM-DD", value_parser = parse_date, help_heading = "Input")]
    start: Option<Date>,
    /// Only activities up to this date
    #[arg(long, value_name = "YYYY-MM-DD", value_parser = parse_date, help_heading = "Input")]
    end: Option<Date>,
    /// Only the last N days, up to --end or today
    #[arg(long, value_name = "N", conflicts_with = "start",
          value_parser = clap::value_parser!(i64).range(1..), help_heading = "Input")]
    days: Option<i64>,
    /// Only these comma-separated activity ids
    #[arg(long = "activity", value_name = "ID", help_heading = "Input")]
    activity: Option<String>,
    /// Activity types to analyze: 'all' or comma-separated keys; types come from summary.csv
    #[arg(long = "type", value_name = "TYPE", default_value = DEFAULT_TYPES, help_heading = "Input")]
    types: String,
    /// Worker threads used to parse the files [default: the CPU count, at most 8]; 1 runs single-threaded
    #[arg(
        long,
        value_name = "N",
        allow_negative_numbers = true,
        help_heading = "Input"
    )]
    jobs: Option<i64>,

    /// How to tell walk from run
    #[arg(long, value_enum, default_value_t = Method::Hybrid, help_heading = "Classification")]
    method: Method,
    /// Strides/min at or above which a segment is a run
    #[arg(long, value_name = "SPM", default_value_t = DEFAULT_CADENCE_THRESHOLD,
          help_heading = "Classification")]
    cadence_threshold: i64,
    /// Pace cutoff as M:SS or seconds [default: 9:00/km]
    #[arg(long, value_name = "M:SS", value_parser = parse_pace, help_heading = "Classification")]
    pace_threshold: Option<i64>,

    /// Seconds of track per segment
    #[arg(long, value_name = "S", default_value_t = DEFAULT_WINDOW, help_heading = "Segmentation")]
    window: i64,
    /// Ignore segments shorter than this many seconds
    #[arg(long, value_name = "S", default_value_t = DEFAULT_MIN_SEGMENT,
          help_heading = "Segmentation")]
    min_segment: i64,
    /// Ignore segments shorter than this many metres
    #[arg(long, value_name = "M", default_value_t = DEFAULT_MIN_DISTANCE_M,
          value_parser = parse_distance, help_heading = "Segmentation")]
    min_distance: f64,
    /// A pause longer than this many seconds ends the segment
    #[arg(long, value_name = "S", default_value_t = DEFAULT_GAP_S, help_heading = "Segmentation")]
    gap: i64,
    /// Exclude segments slower than this from the stats, e.g. 15:00 (filters out standing about)
    #[arg(long, value_name = "M:SS", value_parser = parse_pace, help_heading = "Segmentation")]
    max_pace: Option<i64>,
    /// Exclude segments faster than this from the stats, e.g. 3:00 (drops GPS glitches)
    #[arg(long, value_name = "M:SS", value_parser = parse_pace, help_heading = "Segmentation")]
    min_pace: Option<i64>,

    /// Passes shorter than this many seconds join a neighbour
    #[arg(long, value_name = "S", default_value_t = DEFAULT_MIN_PASS_S, help_heading = "Detail")]
    min_pass: i64,
    /// Seconds of rolling median applied to cadence
    #[arg(long, value_name = "S", default_value_t = DEFAULT_SMOOTH_S, help_heading = "Detail")]
    smooth: i64,

    /// Maximum heart rate, needed for HR zones, TRIMP and pace in zone 3
    #[arg(long, value_name = "BPM", value_parser = clap::value_parser!(i64).range(100..=250),
          help_heading = "Metrics")]
    hr_max: Option<i64>,

    /// Where analysis.csv and segments.csv (passes.csv with --show detail, sessions.csv and weeks.csv with the metric tables) are written
    #[arg(long, value_name = "DIR", default_value = DEFAULT_REPORT_DIR, help_heading = "Output")]
    report_dir: PathBuf,
    /// Also report cadence mean + stdev in strides/min
    #[arg(long, help_heading = "Output")]
    cadence_stats: bool,
    /// Report tables to print: 'all' or comma-separated activities,weekly,overall,detail,summary,progress,zones,fitness,efforts; detail lists every run/walk pass, split on cadence
    #[arg(long, value_name = "TABLES", value_parser = Tables::parse,
          default_value = "activities,weekly,overall", help_heading = "Output")]
    show: Tables,
    /// Print the report only
    #[arg(long, help_heading = "Output")]
    no_csv: bool,
    /// Serve the HTML report on 127.0.0.1 instead of reporting, with route cards you can rename; names are saved to the --route-names file
    #[arg(long, help_heading = "Output")]
    serve: bool,
    /// Port for --serve
    #[arg(long, value_name = "N", default_value_t = DEFAULT_PORT,
          value_parser = clap::value_parser!(u16).range(1..), help_heading = "Output")]
    port: u16,
    /// Ask for a name for each route run in the range (default: the last 30 days) and write them to the --route-names file instead of reporting
    #[arg(long, help_heading = "Output")]
    name_routes: bool,
    /// Also write the metrics as one self-contained HTML page with charts, whatever --show picks
    #[arg(long, value_name = "PATH", help_heading = "Output")]
    html: Option<PathBuf>,
    /// Add a satellite map to the HTML report, with a button to turn it off; the page then holds GPS positions and fetches imagery from EOX when opened
    #[arg(long, help_heading = "Output")]
    map: bool,
}

impl Cli {
    fn into_args(self) -> Args {
        Args {
            out: self.out,
            start: self.start,
            end: self.end,
            types: if self.types == "all" {
                None
            } else {
                list(&self.types)
            },
            // Under 1 runs single-threaded; clamped before the cast so -3 does not wrap.
            jobs: self.jobs.map_or_else(default_jobs, |n| n.max(1) as usize),
            method: self.method,
            cadence_threshold: self.cadence_threshold,
            pace_threshold: self.pace_threshold.unwrap_or(DEFAULT_PACE_THRESHOLD),
            window: self.window,
            min_segment: self.min_segment,
            min_distance: self.min_distance,
            gap: self.gap,
            // A pace guard of 0 means "not set".
            max_pace: self.max_pace.filter(|v| *v != 0),
            min_pace: self.min_pace.filter(|v| *v != 0),
            report_dir: self.report_dir,
            cadence_stats: self.cadence_stats,
            no_csv: self.no_csv,
            show: self.show,
            days: self.days,
            activities: self.activity.as_deref().and_then(list),
            min_pass: self.min_pass,
            smooth: self.smooth,
            hr_max: self.hr_max,
            html: self.html,
            map: self.map,
            route_names: self.route_names,
            name_routes: self.name_routes,
            serve: self.serve,
            port: self.port,
        }
    }
}

pub fn help() -> String {
    Cli::command().render_help().to_string()
}

/// Parse argv (without the program name).
pub fn parse(argv: &[String]) -> Result<Args, CliError> {
    parse_with_today(argv, Date::today())
}

/// `parse`, with the date `--days` counts back from when there is no `--end`.
pub fn parse_with_today(argv: &[String], today: Date) -> Result<Args, CliError> {
    let cli = Cli::try_parse_from(
        std::iter::once("garmin-analyze").chain(argv.iter().map(String::as_str)),
    )?;
    let mut args = cli.into_args();

    if args.map && args.html.is_none() && !args.serve {
        return Err(Cli::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "--map needs --html or --serve",
            )
            .into());
    }
    if let Some(days) = args.days {
        args.start = Some(args.end.unwrap_or(today).minus_days(days - 1));
    }
    if args.name_routes && args.start.is_none() {
        args.start = Some(
            args.end
                .unwrap_or(today)
                .minus_days(DEFAULT_NAMING_DAYS - 1),
        );
    }
    Ok(args)
}
