//! The report as one self-contained HTML page, with its charts as inline SVG.
//!
//! The page loads nothing from the network and holds no latitude or longitude:
//! route drawings are fitted to the picture (src/map.rs). The exception is --map,
//! which adds positions and an embedded Leaflet for satellite maps, fetching tiles
//! while the page's button is on, as it is at first. Every range-dependent
//! section is rendered once per range, and the toggle in the nav picks which one
//! shows.

use std::collections::BTreeMap;
use std::path::Path;

use crate::analyze::{RUN, WALK};
use crate::args::Args;
use crate::dt::Date;
use crate::format::{change, duration_or_dash, fmt_duration, fmt_pace};
use crate::map::{routes_section, session_map};
use crate::metrics::{
    EFFORT_DISTANCES_M, Effort, Metrics, RUNNING_EFFORT_SHARE, Session, Week, ZONES, fastest,
    km_load, prediction_source, previous_weeks, riegel, sessions_in_range, trimp_load,
    typical_gain, vdot,
};
use crate::num::mean_of;
use crate::passes::{Pass, build_passes};

const CSS: &str = include_str!("report.css");
const JS: &str = include_str!("report.js");
const MAP_CSS: &str = include_str!("report_map.css");
const MAP_JS: &str = include_str!("report_map.js");
const LEAFLET_JS: &str = include_str!("vendor/leaflet/leaflet.js");
const LEAFLET_CSS: &str = include_str!("vendor/leaflet/leaflet.css");
const LEAFLET_LICENSE: &str = include_str!("vendor/leaflet/LICENSE");

const RECENT_DAYS: i64 = 182;
/// More routes than this on one chart cannot be told apart reliably by colour.
const ROUTES_CHARTED: usize = 3;
/// A line does not join sessions further apart than this.
const LINE_BREAK_DAYS: i64 = 56;

const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 220.0;
const LEFT: f64 = 44.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 16.0;
const BOTTOM: f64 = 24.0;
const MIN_TICK_SPACING: f64 = 64.0;
const GAP: f64 = 2.0;

/// The sessions and weeks of one choice on the range toggle.
struct View<'m, 'a> {
    key: &'static str,
    sessions: Vec<&'m Session<'a>>,
    weeks: Vec<&'m Week>,
}

/// Write the page, creating its directory if needed.
pub fn write_report(
    path: &Path,
    metrics: &Metrics,
    args: &Args,
    reference: Date,
) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, render(metrics, args, reference))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The whole page.
pub fn render(metrics: &Metrics, args: &Args, reference: Date) -> String {
    let sessions = sessions_in_range(metrics, args);
    let weeks: Vec<&Week> = metrics.weeks.iter().filter(|w| w.in_range(args)).collect();
    let cutoff = reference.minus_days(RECENT_DAYS);
    let recent = View {
        key: "recent",
        sessions: sessions
            .iter()
            .copied()
            .filter(|s| s.activity.date >= cutoff)
            .collect(),
        weeks: weeks
            .iter()
            .copied()
            .filter(|w| w.monday.plus_days(6) >= cutoff)
            .collect(),
    };
    let default_range = if recent.sessions.is_empty() {
        "all"
    } else {
        "recent"
    };
    let all = View {
        key: "all",
        sessions: sessions.clone(),
        weeks,
    };
    let views = [recent, all];
    let routes = charted_routes(&sessions);

    let map_head = if args.map {
        format!(
            "<!-- Leaflet 1.9.4\n{LEAFLET_LICENSE}-->\n<style>\n{LEAFLET_CSS}{MAP_CSS}</style>\n"
        )
    } else {
        String::new()
    };
    let mut page = String::new();
    page.push_str(&format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Running report</title>\n{map_head}<style>\n{CSS}</style>\n</head>\n\
         <body data-range=\"{default_range}\"{editable}>\n",
        editable = if args.serve { " data-editable" } else { "" }
    ));
    page.push_str(&header(&sessions, args));
    page.push_str(&nav(default_range, args.map));
    page.push_str("<main>\n");
    page.push_str(&overview(metrics, args, reference));

    page.push_str(&section("progress", "Progress", |out| {
        for view in &views {
            out.push_str(&in_view(view, &progress(view)));
        }
    }));
    page.push_str(&section("load", "HR and load", |out| match args.hr_max {
        Some(bpm) => {
            out.push_str(&format!(
                "<p class=\"note\">Zones are 50-60, 60-70, 70-80, 80-90 and 90+% of {bpm} bpm.</p>\n"
            ));
            for view in &views {
                out.push_str(&in_view(view, &load(view)));
            }
        }
        None => out.push_str(
            "<p class=\"note\">Needs --hr-max: it is not guessed, because a sensor spike would \
             shift every zone.</p>\n",
        ),
    }));
    page.push_str(&section("fitness", "Fitness", |out| {
        for view in &views {
            out.push_str(&in_view(view, &fitness(view, &routes)));
        }
    }));
    page.push_str(&section("efforts", "Efforts", |out| {
        for view in &views {
            out.push_str(&in_view(view, &efforts(view)));
        }
        out.push_str(&predictions(metrics, reference));
    }));
    page.push_str(&section("routes", "Routes", |out| {
        let naming = if args.serve {
            "Rename a route with its \u{270e} button."
        } else {
            "Name a route by adding <code>&lt;activity id&gt; = &lt;name&gt;</code> to routes.txt \
             in the downloads folder; hover a card for an id."
        };
        out.push_str(&format!(
            "<p class=\"note\">Each route is drawn from its newest session, north up. Click a card \
             for its sessions. {naming}</p>\n"
        ));
        for view in &views {
            out.push_str(&in_view(
                view,
                &routes_section(&view.sessions, &routes, args.map),
            ));
        }
    }));
    page.push_str(&section("sessions", "Sessions", |out| {
        out.push_str(&sessions_table(&sessions, cutoff, args));
    }));
    page.push_str("</main>\n<div id=\"tip\" hidden></div>\n");
    if args.map {
        page.push_str(&format!("<script>\n{LEAFLET_JS}</script>\n"));
    }
    page.push_str(&format!("<script>\n{JS}</script>\n"));
    if args.map {
        page.push_str(&format!("<script>\n{MAP_JS}</script>\n"));
    }
    page.push_str("</body>\n</html>\n");
    page
}

/// Text made safe for HTML content and quoted attributes.
pub fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn section(id: &str, title: &str, body: impl FnOnce(&mut String)) -> String {
    let mut out = format!("<section id=\"{id}\">\n<h2>{title}</h2>\n");
    body(&mut out);
    out.push_str("</section>\n");
    out
}

fn in_view(view: &View, body: &str) -> String {
    format!("<div data-range=\"{}\">\n{body}</div>\n", view.key)
}

fn header(sessions: &[&Session], args: &Args) -> String {
    let mut facts = vec![format!("{} sessions", sessions.len())];
    if let (Some(first), Some(last)) = (sessions.first(), sessions.last()) {
        facts.push(format!("{} to {}", first.activity.date, last.activity.date));
    }
    if let Some(bpm) = args.hr_max {
        facts.push(format!("HR max {bpm} bpm"));
    }
    let warning = if args.map {
        "<p>This page holds your GPS positions, so keep it private.</p>\n"
    } else {
        ""
    };
    format!(
        "<header>\n<h1>Running report</h1>\n<p>{}</p>\n{warning}</header>\n",
        esc(&facts.join(" · "))
    )
}

fn nav(default_range: &str, with_map: bool) -> String {
    let links: String = [
        ("overview", "Overview"),
        ("progress", "Progress"),
        ("load", "HR and load"),
        ("fitness", "Fitness"),
        ("efforts", "Efforts"),
        ("routes", "Routes"),
        ("sessions", "Sessions"),
    ]
    .iter()
    .map(|(id, label)| format!("<a href=\"#{id}\">{label}</a>"))
    .collect();
    let map_button = if with_map {
        "<button type=\"button\" id=\"map-toggle\" aria-pressed=\"true\">Satellite map</button>"
    } else {
        ""
    };
    let button = |key: &str, label: &str| {
        format!(
            "<button type=\"button\" data-set-range=\"{key}\" aria-pressed=\"{}\">{label}</button>",
            key == default_range
        )
    };
    format!(
        "<nav>\n<div class=\"links\">{links}</div>\n<div class=\"controls\">{map_button}<div class=\"range\" role=\"group\" \
         aria-label=\"Date range\">{}{}</div></div>\n</nav>\n",
        button("recent", "Last 6 months"),
        button("all", "All")
    )
}

fn card(label: &str, value: &str, detail: &str) -> String {
    format!(
        "<div class=\"card\"><div class=\"label\">{}</div><div class=\"value\">{}</div>\
         <div class=\"detail\">{}</div></div>\n",
        esc(label),
        esc(value),
        esc(detail)
    )
}

fn overview(metrics: &Metrics, args: &Args, reference: Date) -> String {
    let monday = reference.monday();
    let current = metrics.weeks.iter().find(|w| w.monday == monday);
    let baseline = previous_weeks(&metrics.weeks, monday);
    let (year, week) = monday.isocalendar();
    let km = current.map_or(0.0, |w| w.km);
    let run_s = current.map_or(0.0, |w| w.run_s);
    let longest = current.map_or(0.0, |w| w.longest_run_s);
    let longest_km = current.map_or(0.0, |w| w.longest_run_m) / 1000.0;
    let against = |change: String, baseline: Option<String>| match baseline {
        Some(baseline) => format!("{change} against {baseline}"),
        None => "under 4 weeks of history".to_string(),
    };

    let mut cards = String::new();
    cards.push_str(&card(
        "Distance",
        &format!("{km:.2} km"),
        &against(
            change(km, baseline.map(|b| b.km)),
            baseline.map(|b| format!("{:.2} km a week", b.km)),
        ),
    ));
    cards.push_str(&card(
        "Run time",
        &fmt_duration(run_s),
        &against(
            change(run_s, baseline.map(|b| b.run_s)),
            baseline.map(|b| format!("{} a week", fmt_duration(b.run_s))),
        ),
    ));
    cards.push_str(&card(
        "Longest run",
        &format!("{} · {longest_km:.2} km", fmt_duration(longest)),
        &against(
            change(longest, baseline.map(|b| b.longest_run_s)),
            baseline.map(|b| format!("the best of 4 weeks, {}", fmt_duration(b.longest_run_s))),
        ),
    ));
    let load_card = |label: &str, load: crate::metrics::Load, unit: &str| {
        card(
            label,
            &load
                .ratio
                .map_or("n/a".to_string(), |ratio| format!("{ratio:.2}")),
            &format!(
                "last 7 days {:.1} {unit}, previous 4 weeks {:.1} {unit} a week",
                load.acute, load.baseline
            ),
        )
    };
    cards.push_str(&load_card(
        "Distance load",
        km_load(metrics, reference),
        "km",
    ));
    if args.hr_max.is_some() {
        cards.push_str(&load_card(
            "TRIMP load",
            trimp_load(metrics, reference),
            "pts",
        ));
    }
    section("overview", "Overview", |out| {
        out.push_str(&format!(
            "<p class=\"note\">This week so far ({year}-W{week:02}) against the previous 4 weeks. \
             Load is the last 7 days divided by the mean week of the 28 days before them.</p>\n\
             <div class=\"cards\">\n{cards}</div>\n"
        ));
    })
}

fn week_tip(week: &Week, lines: &[String]) -> String {
    let mut tip = week.label();
    if week.sessions == 0 {
        tip.push_str("\nno sessions");
    }
    for line in lines {
        tip.push('\n');
        tip.push_str(line);
    }
    tip
}

fn progress(view: &View) -> String {
    let mut out = String::from("<div class=\"charts\">\n");
    out.push_str(&bar_chart(
        &view.weeks,
        &BarChart {
            title: "Longest continuous run",
            caption: "Minutes per week, the longest run without a walk break.",
            series: &[("Longest run", "s1")],
            overlay: None,
            peak: |v| format!("{v:.0} min"),
        },
        |w| vec![w.longest_run_s / 60.0],
        |w| {
            let lines = if w.longest_run_s > 0.0 {
                vec![format!(
                    "{} · {:.2} km",
                    fmt_duration(w.longest_run_s),
                    w.longest_run_m / 1000.0
                )]
            } else {
                Vec::new()
            };
            week_tip(w, &lines)
        },
    ));
    out.push_str(&bar_chart(
        &view.weeks,
        &BarChart {
            title: "Distance",
            caption: "Kilometres per week.",
            series: &[("Distance", "s1")],
            overlay: Some(("Mean of the previous 4 weeks", |w| w.km_baseline)),
            peak: |v| format!("{v:.1} km"),
        },
        |w| vec![w.km],
        |w| {
            let mut lines = vec![format!("{:.2} km", w.km)];
            if let Some(baseline) = w.km_baseline {
                lines.push(format!("previous 4 weeks {baseline:.2} km a week"));
            }
            week_tip(w, &lines)
        },
    ));
    out.push_str(&bar_chart(
        &view.weeks,
        &BarChart {
            title: "Run and walk time",
            caption: "Minutes per week.",
            series: &[("Run", "s1"), ("Walk", "s2")],
            overlay: None,
            peak: |v| format!("{v:.0} min"),
        },
        |w| vec![w.run_s / 60.0, w.walk_s / 60.0],
        |w| {
            let lines = vec![
                format!("run {}", duration_or_dash(w.run_s)),
                format!("walk {}", duration_or_dash(w.walk_s)),
            ];
            week_tip(w, &lines)
        },
    ));
    out.push_str("</div>\n");

    let rows: String = view
        .weeks
        .iter()
        .rev()
        .map(|w| {
            let longest_km = (w.longest_run_m > 0.0).then_some(w.longest_run_m / 1000.0);
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
                 <td>{:.2}</td><td>{}</td></tr>\n",
                w.label(),
                w.sessions,
                duration_or_dash(w.longest_run_s),
                opt(longest_km, |v| format!("{v:.2}")),
                duration_or_dash(w.run_s),
                duration_or_dash(w.walk_s),
                opt(w.run_share(), |v| format!("{v:.0}")),
                w.km,
                esc(&change(w.km, w.km_baseline)),
            )
        })
        .collect();
    out.push_str(&details_table(
        "Weekly numbers",
        &[
            "Week",
            "Sessions",
            "Longest",
            "Longest km",
            "Run",
            "Walk",
            "Run %",
            "Km",
            "vs 4 weeks",
        ],
        &rows,
    ));
    out
}

fn load(view: &View) -> String {
    const SERIES: [(&str, &str); ZONES] = [
        ("Zone 1", "z1"),
        ("Zone 2", "z2"),
        ("Zone 3", "z3"),
        ("Zone 4", "z4"),
        ("Zone 5", "z5"),
    ];
    let mut out = String::from("<div class=\"charts\">\n");
    out.push_str(&bar_chart(
        &view.weeks,
        &BarChart {
            title: "Time in HR zones",
            caption: "Minutes per week, over the whole of each session.",
            series: &SERIES,
            overlay: None,
            peak: |v| format!("{v:.0} min"),
        },
        |w| w.zones_all.iter().map(|s| s / 60.0).collect(),
        |w| {
            let lines: Vec<String> = w
                .zones_all
                .iter()
                .enumerate()
                .map(|(i, s)| format!("zone {} {:.0} min", i + 1, s / 60.0))
                .collect();
            week_tip(w, &lines)
        },
    ));
    out.push_str(&bar_chart(
        &view.weeks,
        &BarChart {
            title: "TRIMP",
            caption: "Training load per week: minutes in each zone times the zone number.",
            series: &[("TRIMP", "s1")],
            overlay: None,
            peak: |v| format!("{v:.0}"),
        },
        |w| vec![w.trimp.unwrap_or(0.0)],
        |w| week_tip(w, &[opt(w.trimp, |v| format!("TRIMP {v:.0}"))]),
    ));
    out.push_str("</div>\n");

    let minutes = |zones: &[f64; ZONES]| -> String {
        zones
            .iter()
            .map(|s| format!("<td>{:.0}</td>", s / 60.0))
            .collect()
    };
    let rows: String = view
        .weeks
        .iter()
        .rev()
        .map(|w| {
            format!(
                "<tr><td>{}</td>{}{}<td>{}</td></tr>\n",
                w.label(),
                minutes(&w.zones_all),
                minutes(&w.zones_run),
                opt(w.trimp, |v| format!("{v:.0}"))
            )
        })
        .collect();
    out.push_str(&details_table(
        "Weekly zone minutes",
        &[
            "Week", "All Z1", "Z2", "Z3", "Z4", "Z5", "Run Z1", "Z2", "Z3", "Z4", "Z5", "TRIMP",
        ],
        &rows,
    ));
    out
}

/// The most used routes, each with the colour it keeps in every range.
fn charted_routes(sessions: &[&Session]) -> Vec<(usize, &'static str)> {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for route in sessions
        .iter()
        .filter(|s| s.run_s > 0.0)
        .filter_map(|s| s.route)
    {
        *counts.entry(route).or_default() += 1;
    }
    let mut ranked: Vec<(usize, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(ROUTES_CHARTED)
        .zip(["s1", "s2", "s3"])
        .map(|((route, _), class)| (route, class))
        .collect()
}

fn fitness(view: &View, routes: &[(usize, &'static str)]) -> String {
    let running: Vec<&Session> = view
        .sessions
        .iter()
        .copied()
        .filter(|s| s.run_s > 0.0)
        .collect();
    let series: Vec<Series> = routes
        .iter()
        .map(|&(route, class)| {
            let rows: Vec<&Session> = running
                .iter()
                .copied()
                .filter(|s| s.route == Some(route))
                .collect();
            let gain =
                typical_gain(&rows).map_or(String::new(), |g| format!(", about {g:.0} m climb"));
            Series {
                // A route with no sessions in this view is left out of the legend.
                name: format!(
                    "{}{gain}",
                    rows.first().map_or("", |s| s.route_name.as_str())
                ),
                class,
                points: rows
                    .iter()
                    .filter_map(|s| {
                        s.efficiency.map(|ef| Point {
                            date: s.activity.date,
                            y: ef,
                            tip: format!(
                                "{}\n{}\nefficiency {ef:.2}",
                                s.activity.date, s.route_name
                            ),
                        })
                    })
                    .collect(),
            }
        })
        .collect();

    let mut out = String::from("<div class=\"charts\">\n");
    out.push_str(&line_chart(
        "Efficiency",
        "Metres per minute for each heartbeat per minute, on the 3 most used routes. Higher is better.",
        &series,
        &TWO_DECIMALS,
    ));

    let pace_tip = |s: &Session| {
        format!(
            "{}\npace {}\ngrade-adjusted {}",
            s.activity.date,
            opt(s.run_pace(), fmt_pace),
            opt(s.grade_adjusted_pace, fmt_pace)
        )
    };
    let pace_series = |name: &str, class, value: fn(&Session) -> Option<f64>| Series {
        name: name.to_string(),
        class,
        points: running
            .iter()
            .filter_map(|s| {
                value(s).map(|y| Point {
                    date: s.activity.date,
                    y,
                    tip: pace_tip(s),
                })
            })
            .collect(),
    };
    out.push_str(&line_chart(
        "Run pace",
        "Minutes per kilometre, faster at the top. Grade-adjusted pace is the flat-ground equivalent.",
        &[
            pace_series("Pace", "s1", |s| s.run_pace()),
            pace_series("Grade-adjusted", "s2", |s| s.grade_adjusted_pace),
        ],
        &PACE,
    ));
    out.push_str(&line_chart(
        "HR recovery",
        "Heartbeats per minute lost in the first minute of walking after a run. Higher is better.",
        &[Series {
            name: "Recovery".to_string(),
            class: "s1",
            points: running
                .iter()
                .filter_map(|s| {
                    s.recovery.map(|y| Point {
                        date: s.activity.date,
                        y,
                        tip: format!("{}\nrecovery {y:.0} bpm", s.activity.date),
                    })
                })
                .collect(),
        }],
        &WHOLE,
    ));
    out.push_str("</div>\n");

    let mut all_routes: Vec<Option<usize>> = running.iter().map(|s| s.route).collect();
    all_routes.sort();
    all_routes.dedup();
    let rows: String = all_routes
        .iter()
        .map(|route| {
            let rows: Vec<&Session> = running
                .iter()
                .copied()
                .filter(|s| s.route == *route)
                .collect();
            let efficiency: Vec<f64> = rows.iter().filter_map(|s| s.efficiency).collect();
            let (early, late) = if efficiency.len() >= 6 {
                (
                    mean_of(&efficiency[..3]),
                    mean_of(&efficiency[efficiency.len() - 3..]),
                )
            } else {
                (None, None)
            };
            let two = |v: f64| format!("{v:.2}");
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                esc(&rows[0].route_name),
                rows.len(),
                opt(typical_gain(&rows), |g| format!("{g:.0} m")),
                opt(early, two),
                opt(late, two),
            )
        })
        .collect();
    if !rows.is_empty() {
        out.push_str("<h3>Routes</h3>\n");
        out.push_str(&table(
            &[
                "Route",
                "Sessions",
                "Typical climb",
                "Efficiency, first 3",
                "Efficiency, last 3",
            ],
            &rows,
        ));
    }
    out
}

fn effort_cells(effort: Option<&Effort>) -> String {
    match effort {
        Some(e) => format!(
            "<td>{}</td><td>{}</td><td><a href=\"#s-{}\">{}</a></td><td>{:.0}%</td>",
            fmt_duration(e.time_s),
            fmt_pace(e.pace()),
            esc(&e.activity_id),
            e.date,
            e.run_share * 100.0
        ),
        None => "<td>-</td><td>-</td><td>-</td><td>-</td>".to_string(),
    }
}

fn efforts(view: &View) -> String {
    let mut out = String::from("<div class=\"charts\">\n");
    let classes = ["s1", "s2", "s3"];
    let series: Vec<Series> = EFFORT_DISTANCES_M
        .iter()
        .enumerate()
        .map(|(i, distance)| Series {
            name: format!("{:.0} km", distance / 1000.0),
            class: classes[i],
            points: view
                .sessions
                .iter()
                .filter_map(|s| s.efforts_any[i].as_ref())
                .map(|e| Point {
                    date: e.date,
                    y: e.pace(),
                    tip: format!(
                        "{}\n{:.0} km in {}\npace {} · {:.0}% running",
                        e.date,
                        distance / 1000.0,
                        fmt_duration(e.time_s),
                        fmt_pace(e.pace()),
                        e.run_share * 100.0
                    ),
                })
                .collect(),
        })
        .collect();
    out.push_str(&line_chart(
        "Best pace in each session",
        "The fastest stretch of each distance per session, walk breaks included. Faster at the top.",
        &series,
        &PACE,
    ));
    out.push_str("</div>\n<h3>Best efforts</h3>\n");

    let share = RUNNING_EFFORT_SHARE * 100.0;
    let rows: String = EFFORT_DISTANCES_M
        .iter()
        .enumerate()
        .map(|(i, distance)| {
            let any = fastest(
                view.sessions
                    .iter()
                    .filter_map(|s| s.efforts_any[i].as_ref()),
            );
            let running = fastest(
                view.sessions
                    .iter()
                    .filter_map(|s| s.efforts_running[i].as_ref()),
            );
            format!(
                "<tr><td>{:.0} km</td>{}{}</tr>\n",
                distance / 1000.0,
                effort_cells(any),
                effort_cells(running)
            )
        })
        .collect();
    let running_header = format!("{share:.0}%+ running");
    out.push_str(&table(
        &[
            "Distance",
            "Any",
            "Pace",
            "Date",
            "Running",
            &running_header,
            "Pace",
            "Date",
            "Running",
        ],
        &rows,
    ));
    out
}

fn predictions(metrics: &Metrics, reference: Date) -> String {
    let share = RUNNING_EFFORT_SHARE * 100.0;
    match prediction_source(&metrics.sessions, reference) {
        Some(source) => {
            let target =
                |metres: f64| fmt_duration(riegel(source.time_s, source.distance_m, metres));
            format!(
                "<h3>Predictions</h3>\n<p class=\"note\">From {:.0} km in {} on {} (a training \
                 effort, so likely slower than a race): VDOT {:.1}, 5 km in {}, 10 km in {}.</p>\n",
                source.distance_m / 1000.0,
                fmt_duration(source.time_s),
                source.date,
                vdot(source.distance_m, source.time_s),
                target(5000.0),
                target(10000.0)
            )
        }
        None => format!(
            "<h3>Predictions</h3>\n<p class=\"note\">No predictions: they need an effort of at \
             least 3 km, {share:.0}% running, in the last 90 days.</p>\n"
        ),
    }
}

fn sessions_table(sessions: &[&Session], cutoff: Date, args: &Args) -> String {
    const COLUMNS: [(&str, &str); 16] = [
        ("Date", "text"),
        ("Name", "text"),
        ("Route", "num"),
        ("Run", "num"),
        ("Walk", "num"),
        ("Km", "num"),
        ("Longest", "num"),
        ("Pace", "num"),
        ("Grade adj.", "num"),
        ("HR", "num"),
        ("Efficiency", "num"),
        ("Decoupling %", "num"),
        ("Zone 3 pace", "num"),
        ("Recovery", "num"),
        ("Climb m", "num"),
        ("TRIMP", "num"),
    ];
    let mut routes: BTreeMap<usize, (&str, usize)> = BTreeMap::new();
    for s in sessions {
        if let Some(route) = s.route {
            routes.entry(route).or_insert((&s.route_name, 0)).1 += 1;
        }
    }
    let options: String = routes
        .iter()
        .map(|(route, (name, count))| {
            format!("<option value=\"{route}\">{} ({count})</option>", esc(name))
        })
        .collect();
    let mut out = format!(
        "<p class=\"note\">Click a session for its route and its run and walk passes, or a column to sort.</p>\n\
         <label class=\"filter\">Route <select id=\"route-filter\"><option value=\"\">All routes\
         </option>{options}</select></label>\n<div class=\"table-wrap\">\n\
         <table id=\"sessions-table\">\n<thead><tr>"
    );
    for (i, (label, kind)) in COLUMNS.iter().enumerate() {
        let sorted = if i == 0 {
            " aria-sort=\"descending\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<th{sorted}><button type=\"button\" data-column=\"{i}\" data-kind=\"{kind}\">{label}\
             </button></th>"
        ));
    }
    out.push_str("</tr></thead>\n");
    for s in sessions.iter().rev() {
        out.push_str(&session_rows(s, cutoff, args, COLUMNS.len()));
    }
    out.push_str("</table>\n</div>\n");
    out
}

/// One session as its own `tbody`, so sorting keeps its passes attached to it.
fn session_rows(s: &Session, cutoff: Date, args: &Args, columns: usize) -> String {
    let id = esc(&s.activity.activity_id);
    let route = s.route.map_or(String::new(), |r| r.to_string());
    let old = if s.activity.date < cutoff {
        " class=\"old\""
    } else {
        ""
    };
    let number = |value: Option<f64>, show: &dyn Fn(f64) -> String| match value {
        Some(v) => format!("<td data-v=\"{v}\">{}</td>", show(v)),
        None => "<td>-</td>".to_string(),
    };
    let whole = |v: f64| format!("{v:.0}");
    let two = |v: f64| format!("{v:.2}");
    let one = |v: f64| format!("{v:.1}");
    let pace = |v: f64| fmt_pace(v);
    let duration = |v: f64| duration_or_dash(v);
    let mut row = format!(
        "<tbody id=\"s-{id}\" data-route=\"{route}\"{old}>\n<tr class=\"session\">\
         <td data-v=\"{date}\"><button type=\"button\" class=\"expand\" aria-expanded=\"false\">\
         {date}</button></td><td class=\"name\">{name}</td>",
        date = s.activity.date,
        name = esc(&s.activity.name),
    );
    row.push_str(&match s.route {
        Some(r) => format!("<td data-v=\"{r}\">{}</td>", esc(&s.route_name)),
        None => format!("<td>{}</td>", esc(&s.route_name)),
    });
    row.push_str(&number(Some(s.run_s), &duration));
    row.push_str(&number(Some(s.walk_s), &duration));
    row.push_str(&number(Some(s.km()), &two));
    row.push_str(&number(Some(s.longest_run_s), &duration));
    row.push_str(&number(s.run_pace(), &pace));
    row.push_str(&number(s.grade_adjusted_pace, &pace));
    row.push_str(&number(s.run_hr, &whole));
    row.push_str(&number(s.efficiency, &two));
    row.push_str(&number(s.decoupling, &one));
    row.push_str(&number(s.zone3_pace, &pace));
    row.push_str(&number(s.recovery, &whole));
    row.push_str(&number(s.elev_gain, &whole));
    row.push_str(&number(s.trimp, &whole));
    let passes = build_passes(s.activity, args);
    row.push_str(&format!(
        "</tr>\n<tr class=\"passes\" hidden><td colspan=\"{columns}\"><div class=\"session-detail\">\
         {}{}</div></td></tr>\n</tbody>\n",
        session_map(s.activity, &passes, args.map),
        passes_table(&passes)
    ));
    row
}

fn passes_table(passes: &[Pass]) -> String {
    if passes.is_empty() {
        return "<p class=\"note\">No passes in this session.</p>".to_string();
    }
    let whole = |v: f64| format!("{v:.0}");
    let mut out = String::from(
        "<table class=\"pass-table\"><thead><tr><th>#</th><th>Class</th><th>At</th><th>Time</th>\
         <th>Km</th><th>Pace</th><th>Cadence</th><th>HR</th><th>HR drift</th><th>Climb m</th>\
         <th>Stride m</th></tr></thead><tbody>",
    );
    for (i, pass) in passes.iter().enumerate() {
        let s = &pass.stats;
        let class = match pass.class {
            RUN => "run",
            WALK => "walk",
            other => other,
        };
        let label = if pass.excluded {
            format!("{class}, excluded")
        } else {
            class.to_string()
        };
        out.push_str(&format!(
            "<tr{dim}><td>{}</td><td><span class=\"key {class}\"></span>{}</td><td>{}</td><td>{}</td>\
             <td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            i + 1,
            esc(&label),
            fmt_duration(s.start_offset_s),
            fmt_duration(s.time_s),
            s.distance_m / 1000.0,
            opt(s.pace.filter(|_| s.pace_reliable), fmt_pace),
            opt(s.cad_mean, whole),
            opt(s.hr_mean, whole),
            opt(s.hr_drift, |v| format!("{v:+.0}")),
            opt(s.elev_gain, whole),
            opt(s.stride_m, |v| format!("{v:.2}")),
            dim = if pass.excluded { " class=\"dim\"" } else { "" },
        ));
    }
    out.push_str("</tbody></table>");
    out
}

fn opt(value: Option<f64>, show: impl Fn(f64) -> String) -> String {
    value.map_or("-".to_string(), show)
}

fn table(headers: &[&str], rows: &str) -> String {
    let head: String = headers
        .iter()
        .map(|h| format!("<th>{}</th>", esc(h)))
        .collect();
    format!(
        "<div class=\"table-wrap\">\n<table>\n<thead><tr>{head}</tr></thead>\n<tbody>\n{rows}\
         </tbody>\n</table>\n</div>\n"
    )
}

fn details_table(summary: &str, headers: &[&str], rows: &str) -> String {
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "<details>\n<summary>{}</summary>\n{}</details>\n",
        esc(summary),
        table(headers, rows)
    )
}

// Charts.

fn plot_width() -> f64 {
    WIDTH - LEFT - RIGHT
}

fn plot_height() -> f64 {
    HEIGHT - TOP - BOTTOM
}

fn empty_chart(title: &str) -> String {
    format!(
        "<figure class=\"chart\"><figcaption><h3>{}</h3></figcaption><p class=\"empty\">No data \
         in this range.</p></figure>\n",
        esc(title)
    )
}

fn frame(title: &str, caption: &str, legend: &str, svg: &str) -> String {
    format!(
        "<figure class=\"chart\">\n<figcaption><h3>{title}</h3><p>{caption}</p>{legend}\
         </figcaption>\n<div class=\"plot\"><svg viewBox=\"0 0 {WIDTH} {HEIGHT}\" role=\"img\" \
         aria-label=\"{title}\">\n{svg}</svg></div>\n</figure>\n",
        title = esc(title),
        caption = esc(caption),
    )
}

/// `kind` is the legend mark: a filled box for bars, a stroke for lines.
fn legend(items: &[(String, &str, &str)]) -> String {
    let entries: String = items
        .iter()
        .map(|(name, class, kind)| {
            format!(
                "<span class=\"entry\"><span class=\"{kind} {class}\"></span>{}</span>",
                esc(name)
            )
        })
        .collect();
    format!("<div class=\"legend\">{entries}</div>")
}

/// The largest round step that gives at most four intervals over `range`.
fn nice_step(range: f64) -> f64 {
    let raw = range / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|step| *step >= raw * 0.999)
        .unwrap_or(10.0 * magnitude)
}

/// A step that falls on round times, for an axis in seconds.
fn time_step(range: f64) -> f64 {
    [5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
        .into_iter()
        .find(|step| range / step <= 4.0)
        .unwrap_or_else(|| nice_step(range))
}

fn month_name(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES[(month.clamp(1, 12) - 1) as usize]
}

/// A linear date scale over the plot, with labels on month starts.
struct TimeAxis {
    from: i64,
    to: i64,
    left: f64,
    width: f64,
}

impl TimeAxis {
    fn x(&self, days: i64) -> f64 {
        self.left + (days - self.from) as f64 / (self.to - self.from) as f64 * self.width
    }

    fn labels(&self, start: Date) -> String {
        let span = (self.to - self.from) as f64;
        let per_month = self.width * 30.4 / span;
        let every = [1u32, 2, 3, 6, 12]
            .into_iter()
            .find(|months| per_month * *months as f64 >= MIN_TICK_SPACING)
            .unwrap_or(12);
        let y = HEIGHT - 6.0;
        let (mut year, mut month) = (start.year, start.month);
        let mut out = String::new();
        let mut first = true;
        loop {
            let date = Date {
                year,
                month,
                day: 1,
            };
            if date.days() > self.to {
                break;
            }
            if date.days() >= self.from && (month - 1) % every == 0 {
                let x = self.x(date.days());
                if x <= WIDTH - RIGHT - 14.0 {
                    let label = if first || month == 1 {
                        format!("{} {year}", month_name(month))
                    } else {
                        month_name(month).to_string()
                    };
                    first = false;
                    out.push_str(&format!(
                        "<text class=\"tick\" x=\"{x:.1}\" y=\"{y}\" text-anchor=\"start\">{label}</text>\n"
                    ));
                }
            }
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        if first {
            out.push_str(&format!(
                "<text class=\"tick\" x=\"{:.1}\" y=\"{y}\" text-anchor=\"start\">{start}</text>\n",
                self.left
            ));
        }
        out
    }
}

/// Gridlines and their labels. `y` maps a value to its height on the plot.
fn grid(ticks: &[f64], y: &dyn Fn(f64) -> f64, show: &dyn Fn(f64) -> String) -> String {
    ticks
        .iter()
        .map(|&tick| {
            let height = y(tick);
            format!(
                "<line class=\"grid\" x1=\"{LEFT}\" x2=\"{:.1}\" y1=\"{height:.1}\" y2=\"{height:.1}\"/>\n\
                 <text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>\n",
                WIDTH - RIGHT,
                LEFT - 6.0,
                height + 4.0,
                esc(&show(tick))
            )
        })
        .collect()
}

/// The name and the weekly value of a line drawn over bars.
type Overlay<'c> = (&'c str, fn(&Week) -> Option<f64>);

struct BarChart<'c> {
    title: &'c str,
    caption: &'c str,
    series: &'c [(&'c str, &'c str)],
    /// A line drawn over the bars, in the same unit.
    overlay: Option<Overlay<'c>>,
    peak: fn(f64) -> String,
}

/// A path for a bar with rounded top corners when `round` is set.
fn bar_path(x: f64, top: f64, width: f64, height: f64, round: bool) -> String {
    let radius = if round {
        4f64.min(width / 2.0).min(height)
    } else {
        0.0
    };
    let bottom = top + height;
    if radius <= 0.0 {
        return format!("M{x:.1} {bottom:.1}V{top:.1}H{:.1}V{bottom:.1}Z", x + width);
    }
    format!(
        "M{x:.1} {bottom:.1}V{:.1}Q{x:.1} {top:.1} {:.1} {top:.1}H{:.1}Q{:.1} {top:.1} {:.1} {:.1}V{bottom:.1}Z",
        top + radius,
        x + radius,
        x + width - radius,
        x + width,
        x + width,
        top + radius
    )
}

/// Stacked bars per week, on one baseline. A week without sessions leaves a gap.
fn bar_chart(
    weeks: &[&Week],
    chart: &BarChart,
    values: impl Fn(&Week) -> Vec<f64>,
    tip: impl Fn(&Week) -> String,
) -> String {
    let totals: Vec<f64> = weeks.iter().map(|w| values(w).iter().sum()).collect();
    let overlay_max = chart.overlay.map_or(0.0, |(_, value)| {
        weeks.iter().filter_map(|w| value(w)).fold(0.0, f64::max)
    });
    let peak = totals.iter().copied().fold(0.0, f64::max);
    let (Some(first), Some(last)) = (weeks.first(), weeks.last()) else {
        return empty_chart(chart.title);
    };
    if peak <= 0.0 {
        return empty_chart(chart.title);
    }

    let axis = TimeAxis {
        from: first.monday.days(),
        to: last.monday.days() + 7,
        left: LEFT,
        width: plot_width(),
    };
    let band = plot_width() / weeks.len() as f64;
    let bar_width = (band - GAP).max(band * 0.6).min(24.0);

    let step = nice_step(peak.max(overlay_max));
    let top_value = (peak.max(overlay_max) / step).ceil() * step;
    let scale = plot_height() / top_value;
    let baseline = TOP + plot_height();
    let y = |value: f64| baseline - value * scale;
    let count = (top_value / step).round() as usize;
    let ticks: Vec<f64> = (0..=count).map(|i| i as f64 * step).collect();

    let mut svg = grid(&ticks, &y, &|v| format!("{v:.0}"));
    svg.push_str(&axis.labels(first.monday));
    let mut peak_label: Option<(f64, f64)> = None;
    for (week, total) in weeks.iter().zip(&totals) {
        let slot = axis.x(week.monday.days());
        let x = slot + (band - bar_width) / 2.0;
        let parts = values(week);
        let last_part = parts.iter().rposition(|v| *v > 0.0);
        let mut height_so_far = 0.0;
        for (i, (value, (_, class))) in parts.iter().zip(chart.series).enumerate() {
            if *value <= 0.0 {
                continue;
            }
            let height = value * scale;
            let is_top = last_part == Some(i);
            let drawn = if is_top {
                height
            } else {
                (height - GAP).max(1.0)
            };
            let top = baseline - height_so_far - drawn;
            svg.push_str(&format!(
                "<path class=\"{class}\" d=\"{}\"/>\n",
                bar_path(x, top, bar_width, drawn, is_top)
            ));
            height_so_far += height;
        }
        if *total == peak && peak_label.is_none() {
            peak_label = Some((x + bar_width / 2.0, y(*total)));
        }
    }
    if let Some((_, value)) = &chart.overlay {
        let mut path = String::new();
        let mut pen_down = false;
        for week in weeks {
            match value(week) {
                Some(v) => {
                    let x = axis.x(week.monday.days()) + band / 2.0;
                    path.push_str(&format!(
                        "{}{x:.1} {:.1}",
                        if pen_down { "L" } else { "M" },
                        y(v)
                    ));
                    pen_down = true;
                }
                None => pen_down = false,
            }
        }
        svg.push_str(&format!("<path class=\"baseline-line\" d=\"{path}\"/>\n"));
    }
    svg.push_str(&format!(
        "<line class=\"axis\" x1=\"{LEFT}\" x2=\"{:.1}\" y1=\"{baseline:.1}\" y2=\"{baseline:.1}\"/>\n",
        WIDTH - RIGHT
    ));
    if let Some((x, top)) = peak_label {
        let x = x.clamp(LEFT + 20.0, WIDTH - RIGHT - 20.0);
        svg.push_str(&format!(
            "<text class=\"peak\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>\n",
            (top - 5.0).max(11.0),
            esc(&(chart.peak)(peak))
        ));
    }
    for week in weeks {
        svg.push_str(&format!(
            "<rect class=\"hit\" x=\"{:.1}\" y=\"{TOP}\" width=\"{band:.1}\" height=\"{:.1}\" data-tip=\"{}\"/>\n",
            axis.x(week.monday.days()),
            plot_height(),
            esc(&tip(week))
        ));
    }

    let mut items: Vec<(String, &str, &str)> = Vec::new();
    if chart.series.len() >= 2 || chart.overlay.is_some() {
        for (name, class) in chart.series {
            items.push((name.to_string(), class, "box"));
        }
    }
    if let Some((name, _)) = &chart.overlay {
        items.push((name.to_string(), "neutral", "stroke"));
    }
    let keys = if items.is_empty() {
        String::new()
    } else {
        legend(&items)
    };
    frame(chart.title, chart.caption, &keys, &svg)
}

struct Point {
    date: Date,
    y: f64,
    tip: String,
}

struct Series {
    name: String,
    class: &'static str,
    points: Vec<Point>,
}

struct YAxis {
    /// Smaller values at the top, as with pace.
    invert: bool,
    /// Values are seconds, so the ticks fall on round times.
    seconds: bool,
    show: fn(f64) -> String,
}

const PACE: YAxis = YAxis {
    invert: true,
    seconds: true,
    show: fmt_pace,
};

const TWO_DECIMALS: YAxis = YAxis {
    invert: false,
    seconds: false,
    show: |v| format!("{v:.2}"),
};

const WHOLE: YAxis = YAxis {
    invert: false,
    seconds: false,
    show: |v| format!("{v:.0}"),
};

/// Lines with a marker on every session, on one date scale and one value axis.
fn line_chart(title: &str, caption: &str, series: &[Series], axis_y: &YAxis) -> String {
    let points = || series.iter().flat_map(|s| s.points.iter());
    let (Some(min_date), Some(max_date)) = (
        points().map(|p| p.date).min(),
        points().map(|p| p.date).max(),
    ) else {
        return empty_chart(title);
    };
    let low = points().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let high = points().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let range = (high - low).max(if axis_y.seconds {
        20.0
    } else {
        high.abs() * 0.1 + 0.01
    });
    let step = if axis_y.seconds {
        time_step(range)
    } else {
        nice_step(range)
    };
    let bottom_value = (low / step).floor() * step;
    let top_value = ((high / step).ceil() * step).max(bottom_value + step);
    let count = ((top_value - bottom_value) / step).round() as usize;
    let ticks: Vec<f64> = (0..=count)
        .map(|i| bottom_value + i as f64 * step)
        .collect();
    let fraction = |value: f64| (value - bottom_value) / (top_value - bottom_value);
    let y = |value: f64| {
        let share = if axis_y.invert {
            1.0 - fraction(value)
        } else {
            fraction(value)
        };
        TOP + plot_height() * (1.0 - share)
    };

    // A little room either side, so an end marker does not sit on the plot's edge.
    let pad = ((max_date.days() - min_date.days()) / 40).max(3);
    let axis = TimeAxis {
        from: min_date.days() - pad,
        to: max_date.days() + pad,
        left: LEFT,
        width: plot_width(),
    };

    let mut svg = grid(&ticks, &y, &axis_y.show);
    svg.push_str(&axis.labels(min_date.minus_days(pad)));
    let baseline = TOP + plot_height();
    svg.push_str(&format!(
        "<line class=\"axis\" x1=\"{LEFT}\" x2=\"{:.1}\" y1=\"{baseline:.1}\" y2=\"{baseline:.1}\"/>\n",
        WIDTH - RIGHT
    ));
    for s in series {
        let mut path = String::new();
        let mut previous: Option<Date> = None;
        for point in &s.points {
            let joined = previous.is_some_and(|p| point.date.days() - p.days() <= LINE_BREAK_DAYS);
            path.push_str(&format!(
                "{}{:.1} {:.1}",
                if joined { "L" } else { "M" },
                axis.x(point.date.days()),
                y(point.y)
            ));
            previous = Some(point.date);
        }
        if !path.is_empty() {
            svg.push_str(&format!(
                "<path class=\"line {}\" d=\"{path}\"/>\n",
                s.class
            ));
        }
    }
    for s in series {
        for point in &s.points {
            svg.push_str(&format!(
                "<circle class=\"dot {}\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"4\"/>\n",
                s.class,
                axis.x(point.date.days()),
                y(point.y)
            ));
        }
    }
    for s in series {
        for point in &s.points {
            svg.push_str(&format!(
                "<circle class=\"hit\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"12\" data-tip=\"{}\"/>\n",
                axis.x(point.date.days()),
                y(point.y),
                esc(&point.tip)
            ));
        }
    }
    let visible: Vec<(String, &str, &str)> = series
        .iter()
        .filter(|s| !s.points.is_empty())
        .map(|s| (s.name.clone(), s.class, "stroke"))
        .collect();
    let keys = if visible.len() >= 2 {
        legend(&visible)
    } else {
        // One line of several still needs its name, because the colour alone does not say.
        match (visible.first(), series.len()) {
            (Some(only), n) if n >= 2 => legend(std::slice::from_ref(only)),
            _ => String::new(),
        }
    };
    frame(title, caption, &keys, &svg)
}
