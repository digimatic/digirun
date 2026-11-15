//! The HTML report and its route drawings.

mod common;

use garmin_analyze::args::Args;
use garmin_analyze::dt::Date;
use garmin_analyze::html::render;
use garmin_analyze::map::Frame;
use garmin_analyze::metrics::build;

use common::*;

#[test]
fn html_escapes_activity_names() {
    let run = run_on("1", "<script>alert('x')</script> & co", 3);
    let args = html_args(None);
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(!page.contains("<script>alert"));
    assert!(page.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; co"));
}

#[test]
fn html_explains_a_missing_hr_max() {
    let run = run_on("1", "Run", 3);
    let without = html_args(None);
    let page = render(&build(&[&run], &without), &without, august(3));
    assert!(page.contains("Needs --hr-max"));
    assert!(!page.contains("Time in HR zones"));

    let with = html_args(Some(186));
    let page = render(&build(&[&run], &with), &with, august(3));
    assert!(!page.contains("Needs --hr-max"));
    assert!(page.contains("Time in HR zones"));
    assert!(page.contains("TRIMP load"));
}

#[test]
fn html_keeps_empty_weeks_in_the_charts() {
    let (first, second) = (run_on("1", "Run", 3), run_on("2", "Run", 24));
    let args = html_args(None);
    let page = render(&build(&[&first, &second], &args), &args, august(24));
    // Monday 3 to Monday 24 August, with two weeks between them without a session.
    assert!(page.contains("2026-W33\nno sessions"));
    assert!(page.contains("2026-W34\nno sessions"));
}

#[test]
fn html_handles_sessions_without_hr_or_running() {
    let mut walk = activity(&track(&[(300, 50.0, 1.2)]));
    walk.name = "Walk".to_string();
    for point in &mut walk.points {
        point.hr = None;
    }
    let args = html_args(Some(186));
    let page = render(&build(&[&walk], &args), &args, august(7));
    assert!(page.contains("No data in this range."));
    assert!(page.contains("id=\"s-1\""));
}

#[test]
fn html_shows_an_empty_range_without_failing() {
    let args = html_args(None);
    let metrics = build(&[], &args);
    let page = render(&metrics, &args, august(7));
    assert!(page.contains("0 sessions"));
    assert!(page.contains("body data-range=\"all\""));
}

#[test]
fn html_defaults_to_all_when_nothing_is_recent() {
    let run = run_on("1", "Run", 3);
    let args = html_args(None);
    let later = Date {
        year: 2027,
        month: 8,
        day: 3,
    };
    let page = render(&build(&[&run], &args), &args, later);
    assert!(page.contains("body data-range=\"all\""));
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("body data-range=\"recent\""));
}

#[test]
fn html_holds_no_positions_or_network_addresses() {
    let mut run = run_on("1", "Run", 3);
    for point in &mut run.points {
        point.lat = 57.7412345;
        point.lon = 12.1298765;
    }
    let args = html_args(Some(186));
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(!page.contains("57.74"));
    assert!(!page.contains("12.12"));
    assert!(!page.contains("http"));
}

#[test]
fn html_adds_the_map_and_positions_only_when_asked() {
    let run = run_from("1", 3, 57.7412345, 12.1298765);
    let plain = html_args(None);
    let page = render(&build(&[&run], &plain), &plain, august(3));
    assert!(!page.contains("data-editable"));
    assert!(!page.contains("map-toggle"));
    assert!(!page.to_lowercase().contains("leaflet"));
    assert!(!page.contains("s2cloudless"));
    assert!(!page.contains("data-track"));

    let with_map = Args {
        map: true,
        ..html_args(None)
    };
    let page = render(&build(&[&run], &with_map), &with_map, august(3));
    assert!(page.contains("id=\"map-toggle\" aria-pressed=\"true\""));
    assert!(page.contains("s2cloudless-2025_3857"));
    assert!(page.contains("EOX IT Services GmbH"));
    assert!(page.contains("[57.74123,12.12988]"));
    assert!(page.contains("keep it private"));
    // The embedded scripts must not end their own script element early.
    assert_eq!(
        page.matches("<script>").count(),
        page.matches("</script>").count()
    );
}

#[test]
fn map_tolerates_a_session_without_a_track() {
    let run = run_on("1", "Run", 3);
    let args = Args {
        map: true,
        ..html_args(None)
    };
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("No GPS track for this session."));
    assert!(!page.contains("data-track"));
}

#[test]
fn route_frame_uses_one_scale_for_both_axes() {
    let square = [(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];
    let frame = Frame::fit(&square, (320.0, 220.0), 20.0).unwrap();
    let (left, bottom) = frame.px((0.0, 0.0));
    let (right, top) = frame.px((1000.0, 1000.0));
    assert!((right - left - (bottom - top)).abs() < 1e-9);
    assert!(top < bottom, "north is up");
    assert!(top >= 20.0 && bottom <= 200.0);
    assert_eq!(Frame::fit(&[], (320.0, 220.0), 20.0), None);
}

#[test]
fn html_draws_routes_without_their_positions() {
    let run = run_from("1", 3, 57.7412345, 12.1298765);
    let args = html_args(None);
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("class=\"card route-card\" href=\"#sessions\" data-route=\"0\""));
    assert!(page.contains("class=\"track run\""));
    assert!(!page.contains("57.7"));
    assert!(!page.contains("12.1298"));
}

#[test]
fn html_notes_a_session_without_a_track() {
    let run = run_on("1", "Run", 3);
    let args = html_args(None);
    let page = render(&build(&[&run], &args), &args, august(3));
    assert!(page.contains("No GPS track for this session."));
    assert!(page.contains("No GPS tracks in this range."));
}

#[test]
fn html_overview_leaves_out_a_faraway_route() {
    let (a, b, far) = (
        run_from("1", 3, 57.74, 12.12),
        run_from("2", 4, 57.75, 12.13),
        run_from("3", 5, 40.0, -3.7),
    );
    let args = html_args(None);
    let page = render(&build(&[&a, &b, &far], &args), &args, august(5));
    assert!(page.contains("1 further away than 15 km left out here"));
    // The faraway route keeps its card.
    assert_eq!(page.matches("data-route=\"2\" title=").count(), 2);
}
