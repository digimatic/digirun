//! Route grouping, naming, routes.txt and the rename server.

mod common;

use std::collections::BTreeSet;

use garmin_analyze::args::Args;
use garmin_analyze::html::render;
use garmin_analyze::metrics::build;
use garmin_analyze::naming::{Change, ask, recent_routes, rewrite};
use garmin_analyze::routes::{
    Naming, assign_routes, describe_route, name_routes, parse_route_names,
};
use garmin_analyze::serve::{Request, State, decode_form, handle};

use common::*;

#[test]
fn routes_group_on_overlap() {
    let cells = |range: std::ops::Range<i64>| range.map(|i| (i, 0)).collect::<BTreeSet<_>>();
    let tracks = [
        cells(0..100),
        cells(0..90),    // 90% of the first
        cells(500..600), // elsewhere
        BTreeSet::new(), // no GPS
        cells(50..150),  // a third of the first
        cells(505..600),
    ];
    assert_eq!(
        assign_routes(&tracks),
        vec![Some(0), Some(0), Some(1), None, Some(2), Some(1)]
    );
}

#[test]
fn routes_are_described_by_shape_and_direction() {
    let square = [
        (0.0, 0.0),
        (0.0, 1000.0),
        (1000.0, 1000.0),
        (1000.0, 0.0),
        (0.0, 0.0),
    ];
    assert_eq!(describe_route(0, &shaped(&square)), "R1 · 4.0 km loop NE");
    let there_and_back = [(0.0, 0.0), (0.0, 2000.0), (0.0, 0.0)];
    assert_eq!(
        describe_route(1, &shaped(&there_and_back)),
        "R2 · 4.0 km out-and-back N"
    );
    let east = [(0.0, 0.0), (3000.0, 0.0)];
    assert_eq!(describe_route(2, &shaped(&east)), "R3 · 3.0 km one-way E");
    assert_eq!(describe_route(3, &run_on("1", "Run", 3)), "R4");
}

#[test]
fn route_names_file_parses() {
    let text = "# my routes\n\n 2 = Home loop \n1=Lake # by the water\n";
    assert_eq!(
        parse_route_names(text).unwrap(),
        vec![
            ("2".to_string(), "Home loop".to_string()),
            ("1".to_string(), "Lake".to_string()),
        ]
    );
    assert_eq!(
        parse_route_names("1 = A\n2 Home\n").unwrap_err(),
        "line 2: expected `<activity id> = <name>`"
    );
    assert!(parse_route_names("= Home").is_err());
    assert!(parse_route_names("2 =").is_err());
}

#[test]
fn any_session_names_its_route() {
    let (a, b, far, untracked) = (
        run_from("1", 3, 57.74, 12.12),
        run_from("2", 4, 57.74, 12.12),
        run_from("3", 5, 40.0, -3.7),
        run_on("4", "Run", 6),
    );
    let args = html_args(None);
    let mut metrics = build(&[&a, &b, &far, &untracked], &args);
    let names =
        parse_route_names("2 = Home loop\n1 = Other\n1 = Home loop\n99 = Gone\n4 = Treadmill\n")
            .unwrap();
    assert_eq!(
        name_routes(&mut metrics, &names),
        Naming {
            unmatched: vec!["99".to_string()],
            conflicts: vec![
                "1 = Other: its route is already named Home loop".to_string(),
                "4 has no GPS track to name".to_string(),
            ],
        }
    );
    let named: Vec<&str> = metrics
        .sessions
        .iter()
        .map(|s| s.route_name.as_str())
        .collect();
    assert_eq!(named[..2], ["Home loop", "Home loop"]);
    assert!(named[2].starts_with("R2 · "), "{}", named[2]);
    assert_eq!(named[3], "no GPS");
    let page = render(&metrics, &args, august(6));
    assert!(page.contains("<div class=\"label\">Home loop</div>"));
    assert!(page.contains("title=\"Name it in routes.txt: 2 = ...\""));
}

#[test]
fn recent_routes_are_the_ones_run_in_range() {
    let runs = four_runs();
    let metrics = named_metrics(&runs);
    let args = Args {
        start: Some(august(4)),
        ..Args::default()
    };
    let routes = recent_routes(&metrics, &args);
    let summary: Vec<(usize, &str, usize, &str)> = routes
        .iter()
        .map(|r| {
            (
                r.index,
                r.current.as_str(),
                r.sessions,
                r.newest_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [(0, "Home", 2, "2"), (1, routes[1].auto.as_str(), 1, "3")]
    );
    assert!(routes[0].auto.starts_with("R1 · "));
    let later = Args {
        start: Some(august(6)),
        ..Args::default()
    };
    assert_eq!(recent_routes(&metrics, &later).len(), 1);
}

#[test]
fn ask_keeps_renames_and_clears() {
    let runs = four_runs();
    let metrics = named_metrics(&runs);
    let routes = recent_routes(&metrics, &Args::default());
    let mut shown = Vec::new();
    let changes = ask(&routes, "-\nCafe # no\nCafe\n".as_bytes(), &mut shown).unwrap();
    assert_eq!(
        changes,
        [
            Change {
                route: 0,
                name: None
            },
            Change {
                route: 1,
                name: Some("Cafe".to_string())
            },
        ]
    );
    let shown = String::from_utf8(shown).unwrap();
    assert!(shown.contains("  name [Home]: "));
    assert!(shown.contains("can't contain #"));
    // Enter keeps; clearing an unnamed route, or end of input, changes nothing.
    assert_eq!(
        ask(&routes, "\n-\n".as_bytes(), Vec::new()).unwrap(),
        [] as [garmin_analyze::naming::Change; 0]
    );
    assert_eq!(
        ask(&routes, "".as_bytes(), Vec::new()).unwrap(),
        [] as [garmin_analyze::naming::Change; 0]
    );
    assert_eq!(
        ask(&routes, "Park\n".as_bytes(), Vec::new()).unwrap().len(),
        1
    );
}

#[test]
fn rewrite_replaces_the_lines_of_changed_routes() {
    let runs = four_runs();
    let metrics = named_metrics(&runs);
    let routes = recent_routes(&metrics, &Args::default());
    let text = "# mine\n1 = Home\n77 = Elsewhere\n";
    let renamed = [
        Change {
            route: 0,
            name: Some("Lake".to_string()),
        },
        Change {
            route: 1,
            name: Some("Madrid".to_string()),
        },
    ];
    assert_eq!(
        rewrite(text, &renamed, &metrics, &routes),
        "# mine\n77 = Elsewhere\n2 = Lake\n3 = Madrid\n"
    );
    let cleared = [Change {
        route: 0,
        name: None,
    }];
    assert_eq!(
        rewrite(text, &cleared, &metrics, &routes),
        "# mine\n77 = Elsewhere\n"
    );
    assert!(rewrite("", &renamed, &metrics, &routes).starts_with("# Route names"));
}

#[test]
fn form_bodies_decode() {
    assert_eq!(
        decode_form("route=3&name=Sj%C3%B6+loop%23").unwrap(),
        [
            ("route".to_string(), "3".to_string()),
            ("name".to_string(), "Sjö loop#".to_string()),
        ]
    );
    assert_eq!(
        decode_form("name=").unwrap(),
        [("name".to_string(), String::new())]
    );
    assert!(decode_form("name=%zz").is_none());
    assert!(decode_form("name=%C3").is_none());
}

#[test]
fn served_report_renames_routes() {
    let runs = four_runs();
    let mut metrics = named_metrics(&runs);
    let path = temp_dir("serve").join("routes.txt");
    std::fs::write(&path, "# mine\n1 = Home\n").unwrap();
    let mut state = State::new(&mut metrics, &Args::default(), path.clone(), august(21));
    let get = Request {
        method: "GET".to_string(),
        path: "/".to_string(),
        ..Request::default()
    };
    let page = String::from_utf8(handle(&get, &mut state).body).unwrap();
    assert!(page.contains("<body data-range=\"recent\" data-editable>"));
    assert!(page.contains("<div class=\"label\">Home</div>"));

    assert_eq!(
        handle(&rename_request("route=0&name=Lake+loop"), &mut state).status,
        204
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# mine\n2 = Lake loop\n"
    );
    let page = String::from_utf8(handle(&get, &mut state).body).unwrap();
    assert!(page.contains("<div class=\"label\">Lake loop</div>"));

    // An empty name goes back to the automatic one.
    assert_eq!(
        handle(&rename_request("route=0&name="), &mut state).status,
        204
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
    let page = String::from_utf8(handle(&get, &mut state).body).unwrap();
    assert!(page.contains("<div class=\"label\">R1 · "));

    assert_eq!(
        handle(&rename_request("route=0&name=A%23B"), &mut state).status,
        400
    );
    assert_eq!(
        handle(&rename_request("route=9&name=X"), &mut state).status,
        400
    );
    let mut unmarked = rename_request("route=0&name=X");
    unmarked.headers.retain(|(n, _)| n != "x-digirun");
    assert_eq!(handle(&unmarked, &mut state).status, 403);
    let mut foreign = rename_request("route=0&name=X");
    foreign.headers[0].1 = "evil.example:8765".to_string();
    assert_eq!(handle(&foreign, &mut state).status, 403);
    let lost = Request {
        path: "/routes.txt".to_string(),
        ..get.clone()
    };
    assert_eq!(handle(&lost, &mut state).status, 404);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
}
