//! Command-line parsing.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use garmin_analyze::args::{Args, Tables, help, parse, parse_with_today};
use garmin_analyze::dt::Date;

use common::*;

#[test]
fn days_counts_back_from_end_or_today() {
    let argv = |values: &[&str]| {
        values
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    };
    let today = Date {
        year: 2026,
        month: 1,
        day: 3,
    };

    let args = parse_with_today(&argv(&["--days", "7"]), today).expect("--days should parse");
    assert_eq!(
        (args.start, args.end, args.days),
        (Date::from_iso("2025-12-28"), None, Some(7))
    );
    let args = parse_with_today(&argv(&["--end", "2026-10-18", "--days=7"]), today)
        .expect("--days with --end should parse");
    assert_eq!(args.start, Date::from_iso("2026-10-12"));

    for bad in [
        &["--days", "0"][..],
        &["--days", "3", "--start", "2026-10-01"][..],
    ] {
        let error = parse_with_today(&argv(bad), today).expect_err("should be refused");
        assert_eq!(error.code, 2);
    }
    // No range at all: every table covers all of time.
    assert_eq!(parse_with_today(&[], today).expect("empty").start, None);
}

#[test]
fn detail_options_parse() {
    let argv: Vec<String> = [
        "--show",
        "detail",
        "--activity",
        "1, 2",
        "--min-pass=30",
        "--smooth",
        "6",
    ]
    .iter()
    .map(std::string::ToString::to_string)
    .collect();
    let args = parse(&argv).expect("the options should parse");

    assert!(args.show.detail);
    assert!(!args.show.segments());
    assert_eq!(
        args.activities,
        Some(BTreeSet::from(["1".to_string(), "2".to_string()]))
    );
    assert_eq!((args.min_pass, args.smooth), (30, 6));
    // --detail was folded into --show.
    assert_eq!(parse(&["--detail".to_string()]).expect_err("gone").code, 2);
}

#[test]
fn show_picks_the_report_tables() {
    let argv = |values: &[&str]| {
        values
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    };

    let args = parse(&argv(&["--show", "weekly, overall"])).expect("the option should parse");
    assert_eq!(
        args.show,
        Tables {
            activities: false,
            weekly: true,
            overall: true,
            ..Tables::none()
        }
    );
    // The default is the segment tables; 'all' adds the others.
    assert_eq!(Args::default().show, Tables::default());
    assert!(!Tables::default().detail && Tables::default().segments());
    assert_eq!(
        parse(&argv(&["--show=all"]))
            .expect("all should parse")
            .show,
        Tables::all()
    );

    let error = parse(&argv(&["--show", "week"])).expect_err("week is not a table");
    assert_eq!(error.code, 2);
    assert!(error.message.contains("invalid choice 'week'"));
    assert!(help().contains("--show <TABLES>"));
    assert!(help().contains("--days <N>"));
}

#[test]
fn name_routes_option_asks_about_the_last_30_days() {
    let today = august(31);
    let args = parse_with_today(&["--name-routes".to_string()], today).unwrap();
    assert!(args.name_routes);
    assert_eq!(args.start, Some(august(2)));
    let days = [
        "--name-routes".to_string(),
        "--days".to_string(),
        "7".to_string(),
    ];
    assert_eq!(
        parse_with_today(&days, today).unwrap().start,
        Some(august(25))
    );
    assert!(help().contains("--name-routes"));
}

#[test]
fn metric_options_parse() {
    let argv = |values: &[&str]| {
        values
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    };

    let args = parse(&argv(&["--show", "summary,zones", "--hr-max=186"])).expect("should parse");
    assert_eq!(args.hr_max, Some(186));
    assert!(args.show.summary && args.show.zones && !args.show.progress);
    assert!(args.show.metrics() && !args.show.segments());
    assert!(!Tables::default().metrics());
    assert!(Tables::all().metrics());
    assert_eq!(
        parse(&argv(&["--hr-max", "20"])).expect_err("too low").code,
        2
    );
    assert_eq!(
        parse(&argv(&["--hr-max", "x"]))
            .expect_err("not a number")
            .code,
        2
    );
    assert!(help().contains("--hr-max <BPM>"));
}

#[test]
fn html_option_parses() {
    let args = parse(&["--html".to_string(), "out/report.html".to_string()]).unwrap();
    assert_eq!(args.html, Some(PathBuf::from("out/report.html")));
    assert_eq!(parse(&[]).unwrap().html, None);
}

#[test]
fn serve_options_parse() {
    let args = parse(&[
        "--serve".to_string(),
        "--port".to_string(),
        "9000".to_string(),
        "--map".to_string(),
    ])
    .unwrap();
    assert!(args.serve && args.map);
    assert_eq!(args.port, 9000);
    assert_eq!(Args::default().port, 8765);
    assert!(parse(&["--port".to_string(), "0".to_string()]).is_err());
    assert!(parse(&["--port".to_string(), "http".to_string()]).is_err());
}

#[test]
fn map_needs_html() {
    let map = "--map".to_string();
    assert!(parse(std::slice::from_ref(&map)).is_err());
    let html = ["--html".to_string(), "r.html".to_string(), map];
    assert!(parse(&html).unwrap().map);
    assert!(!parse(&[]).unwrap().map);
}

#[test]
fn jobs_under_one_runs_single_threaded() {
    for value in ["0", "-3"] {
        let args = parse(&["--jobs".to_string(), value.to_string()]).unwrap();
        assert_eq!(args.jobs, 1, "--jobs {value}");
    }
}
