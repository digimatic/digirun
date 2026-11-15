//! Route drawings for the HTML report.
//!
//! Tracks are written as drawing coordinates fitted to each picture, so the page
//! holds the shape and scale of a route but no latitude or longitude. Only --map
//! adds real positions, for the satellite maps.

use std::collections::BTreeMap;

use crate::analyze::RUN;
use crate::html::esc;
use crate::metrics::{Session, typical_gain};
use crate::model::{Activity, Point};
use crate::num::upper_median;
use crate::passes::Pass;

const METRES_PER_DEGREE: f64 = 111_320.0;
/// A point closer than this to the last one drawn adds nothing but bytes.
const MIN_STEP_PX: f64 = 1.0;
/// A smaller track still gets a readable scale.
const MIN_SPAN_M: f64 = 50.0;
/// How closely a satellite map follows the track, which also sets the file size.
const MAP_STEP_M: f64 = 5.0;
/// A route starting further from the others would shrink them all to a dot.
const OVERVIEW_RADIUS_M: f64 = 15_000.0;

const SESSION_SIZE: (f64, f64) = (320.0, 220.0);
const CARD_SIZE: (f64, f64) = (220.0, 150.0);
const OVERVIEW_SIZE: (f64, f64) = (720.0, 420.0);
const PAD: f64 = 18.0;

fn has_position(point: &Point) -> bool {
    point.lat != 0.0 || point.lon != 0.0
}

/// Metres east and north, with `cos_lat` the east-west scale at the track's latitude.
fn metres(point: &Point, cos_lat: f64) -> (f64, f64) {
    (
        point.lon * METRES_PER_DEGREE * cos_lat,
        point.lat * METRES_PER_DEGREE,
    )
}

fn cos_lat_of(activity: &Activity) -> Option<f64> {
    activity
        .points
        .iter()
        .find(|p| has_position(p))
        .map(|p| p.lat.to_radians().cos())
}

/// Fits metres into a picture at one scale for both axes, north up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    centre: (f64, f64),
    size: (f64, f64),
    /// Pixels per metre.
    pub scale: f64,
}

impl Frame {
    /// None without any points.
    pub fn fit(points: &[(f64, f64)], size: (f64, f64), pad: f64) -> Option<Frame> {
        let first = points.first()?;
        let (mut low, mut high) = (*first, *first);
        for &(x, y) in points {
            low = (low.0.min(x), low.1.min(y));
            high = (high.0.max(x), high.1.max(y));
        }
        let span_x = (high.0 - low.0).max(MIN_SPAN_M);
        let span_y = (high.1 - low.1).max(MIN_SPAN_M);
        Some(Frame {
            centre: ((low.0 + high.0) / 2.0, (low.1 + high.1) / 2.0),
            size,
            scale: ((size.0 - 2.0 * pad) / span_x).min((size.1 - 2.0 * pad) / span_y),
        })
    }

    pub fn px(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.size.0 / 2.0 + (x - self.centre.0) * self.scale,
            self.size.1 / 2.0 - (y - self.centre.1) * self.scale,
        )
    }
}

/// An SVG path through the points that have a position.
fn path(points: &[Point], cos_lat: f64, frame: &Frame) -> String {
    let mut out = String::new();
    let mut last: Option<(f64, f64)> = None;
    let located: Vec<&Point> = points.iter().filter(|p| has_position(p)).collect();
    for (i, point) in located.iter().enumerate() {
        let (x, y) = frame.px(metres(point, cos_lat));
        let far_enough =
            last.is_none_or(|(a, b)| (x - a).abs() >= MIN_STEP_PX || (y - b).abs() >= MIN_STEP_PX);
        if far_enough || i + 1 == located.len() {
            let command = if last.is_some() { "L" } else { "M" };
            out.push_str(&format!("{command}{x:.1} {y:.1}"));
            last = Some((x, y));
        }
    }
    out
}

fn scale_bar(frame: &Frame, size: (f64, f64)) -> String {
    let longest = size.0 / 4.0 / frame.scale;
    let magnitude = 10f64.powf(longest.log10().floor());
    let metres = [5.0, 2.0, 1.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|length| *length <= longest)
        .unwrap_or(magnitude);
    let label = if metres >= 1000.0 {
        format!("{:.0} km", metres / 1000.0)
    } else {
        format!("{metres:.0} m")
    };
    let (x, y) = (8.0, size.1 - 8.0);
    format!(
        "<line class=\"scale\" x1=\"{x}\" x2=\"{:.1}\" y1=\"{y}\" y2=\"{y}\"/>\
         <text class=\"tick\" x=\"{x}\" y=\"{:.1}\">{label}</text>\n",
        x + metres * frame.scale,
        y - 5.0
    )
}

/// `[[lat,lon],...]`, leaving out points within `MAP_STEP_M` of the last one.
fn positions_json(points: &[Point], cos_lat: f64) -> String {
    let located: Vec<&Point> = points.iter().filter(|p| has_position(p)).collect();
    let mut kept: Vec<String> = Vec::new();
    let mut last: Option<(f64, f64)> = None;
    for (i, point) in located.iter().enumerate() {
        let here = metres(point, cos_lat);
        let far_enough = last.is_none_or(|(x, y)| (here.0 - x).hypot(here.1 - y) >= MAP_STEP_M);
        if far_enough || i + 1 == located.len() {
            kept.push(format!("[{:.5},{:.5}]", point.lat, point.lon));
            last = Some(here);
        }
    }
    format!("[{}]", kept.join(","))
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The element a satellite map of this session is made in: `[[kind,points],...]`,
/// where the kind is 0 for a walk, 1 for a run and 2 without passes.
fn session_map_host(activity: &Activity, passes: &[Pass], cos_lat: f64) -> String {
    let parts: Vec<String> = if passes.is_empty() {
        vec![format!("[2,{}]", positions_json(&activity.points, cos_lat))]
    } else {
        passes
            .iter()
            .map(|pass| {
                format!(
                    "[{},{}]",
                    (pass.class == RUN) as u8,
                    positions_json(&activity.points[pass.start..=pass.end], cos_lat)
                )
            })
            .collect()
    };
    format!(
        "<div class=\"map-host\" data-track=\"[{}]\"></div>",
        parts.join(",")
    )
}

fn svg(size: (f64, f64), label: &str, body: &str) -> String {
    format!(
        "<svg class=\"map\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"{}\">\n{body}</svg>\n",
        size.0,
        size.1,
        esc(label)
    )
}

fn located_metres(activity: &Activity, cos_lat: f64) -> Vec<(f64, f64)> {
    activity
        .points
        .iter()
        .filter(|p| has_position(p))
        .map(|p| metres(p, cos_lat))
        .collect()
}

/// A session's track, coloured by run and walk, with its start marked.
pub(crate) fn session_map(activity: &Activity, passes: &[Pass], with_map: bool) -> String {
    let drawing = cos_lat_of(activity).and_then(|cos_lat| {
        let located = located_metres(activity, cos_lat);
        let frame = Frame::fit(&located, SESSION_SIZE, PAD)?;
        let mut body = String::new();
        if passes.is_empty() {
            body.push_str(&format!(
                "<path class=\"track\" d=\"{}\"/>\n",
                path(&activity.points, cos_lat, &frame)
            ));
        }
        for pass in passes {
            let class = if pass.class == RUN { "run" } else { "walk" };
            body.push_str(&format!(
                "<path class=\"track {class}\" d=\"{}\"/>\n",
                path(&activity.points[pass.start..=pass.end], cos_lat, &frame)
            ));
        }
        let (x, y) = frame.px(located[0]);
        body.push_str(&format!(
            "<circle class=\"start\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"5\"/>\n"
        ));
        body.push_str(&scale_bar(&frame, SESSION_SIZE));
        Some(svg(SESSION_SIZE, "Route of this session", &body))
    });
    let (has_map, host) = match cos_lat_of(activity).filter(|_| with_map) {
        Some(cos_lat) => (" has-map", session_map_host(activity, passes, cos_lat)),
        None => ("", String::new()),
    };
    match drawing {
        Some(drawing) => format!(
            "<div class=\"session-map{has_map}\">{drawing}{host}<div class=\"legend\">\
             <span class=\"entry\"><span class=\"stroke s1\"></span>Run</span>\
             <span class=\"entry\"><span class=\"stroke s2\"></span>Walk</span>\
             <span class=\"entry\"><span class=\"key start-key\"></span>Start</span></div></div>"
        ),
        None => "<p class=\"note\">No GPS track for this session.</p>".to_string(),
    }
}

/// The sessions of one route.
struct Route<'m, 'a> {
    index: usize,
    sessions: Vec<&'m Session<'a>>,
}

impl<'a> Route<'_, 'a> {
    /// The route is drawn from its newest session.
    fn newest(&self) -> &'a Activity {
        self.sessions[self.sessions.len() - 1].activity
    }

    fn name(&self) -> &str {
        &self.sessions[0].route_name
    }

    fn colour(&self, colours: &[(usize, &'static str)]) -> &'static str {
        colours
            .iter()
            .find(|(route, _)| *route == self.index)
            .map_or("other", |(_, class)| class)
    }

    fn typical_km(&self) -> f64 {
        let mut km: Vec<f64> = self.sessions.iter().map(|s| s.km()).collect();
        km.sort_by(f64::total_cmp);
        km[km.len() / 2]
    }

    fn summary(&self) -> String {
        let count = self.sessions.len();
        let plural = if count == 1 { "" } else { "s" };
        let climb = typical_gain(&self.sessions)
            .map_or(String::new(), |gain| format!(", {gain:.0} m climb"));
        format!(
            "{count} session{plural}, {:.1} km{climb}",
            self.typical_km()
        )
    }
}

/// Most used first, as the sessions are oldest first.
fn routes_of<'m, 'a>(sessions: &[&'m Session<'a>]) -> Vec<Route<'m, 'a>> {
    let mut groups: BTreeMap<usize, Vec<&'m Session<'a>>> = BTreeMap::new();
    for session in sessions {
        if let Some(route) = session.route {
            groups.entry(route).or_default().push(session);
        }
    }
    let mut routes: Vec<Route> = groups
        .into_iter()
        .map(|(index, sessions)| Route { index, sessions })
        .collect();
    routes.sort_by(|a, b| {
        b.sessions
            .len()
            .cmp(&a.sessions.len())
            .then(a.index.cmp(&b.index))
    });
    routes
}

/// All the routes on one picture, leaving out any that start far from the rest.
fn overview_map(routes: &[Route], colours: &[(usize, &'static str)], with_map: bool) -> String {
    let starts: Vec<Point> = routes
        .iter()
        .filter_map(|r| r.newest().points.iter().find(|p| has_position(p)).cloned())
        .collect();
    if starts.is_empty() {
        return String::new();
    }
    // starts is not empty, so there always is a median.
    let middle = |values: Vec<f64>| upper_median(values).unwrap_or_default();
    let cos_lat = middle(starts.iter().map(|p| p.lat).collect())
        .to_radians()
        .cos();
    let centre = (
        middle(starts.iter().map(|p| metres(p, cos_lat).0).collect()),
        middle(starts.iter().map(|p| metres(p, cos_lat).1).collect()),
    );
    let near = |route: &&Route| {
        route
            .newest()
            .points
            .iter()
            .find(|p| has_position(p))
            .is_some_and(|p| {
                let (x, y) = metres(p, cos_lat);
                (x - centre.0).hypot(y - centre.1) <= OVERVIEW_RADIUS_M
            })
    };
    let shown: Vec<&Route> = routes.iter().filter(near).collect();
    let located: Vec<(f64, f64)> = shown
        .iter()
        .flat_map(|r| located_metres(r.newest(), cos_lat))
        .collect();
    let Some(frame) = Frame::fit(&located, OVERVIEW_SIZE, PAD) else {
        return String::new();
    };

    // The least used go first, so the routes in colour stay on top.
    let mut body = String::new();
    let mut hits = String::new();
    for route in shown.iter().rev() {
        let d = path(&route.newest().points, cos_lat, &frame);
        body.push_str(&format!(
            "<path class=\"track {}\" d=\"{d}\"/>\n",
            route.colour(colours)
        ));
        hits.push_str(&format!(
            "<path class=\"track-hit\" d=\"{d}\" data-tip=\"{}\"/>\n",
            esc(&format!("{}\n{}", route.name(), route.summary()))
        ));
    }
    body.push_str(&hits);
    body.push_str(&scale_bar(&frame, OVERVIEW_SIZE));

    let entries: String = shown
        .iter()
        .filter(|r| r.colour(colours) != "other")
        .map(|r| {
            format!(
                "<span class=\"entry\"><span class=\"stroke {}\"></span>{}</span>",
                r.colour(colours),
                esc(r.name())
            )
        })
        .collect();
    let others = if shown.iter().any(|r| r.colour(colours) == "other") {
        "<span class=\"entry\"><span class=\"stroke other\"></span>Other routes</span>"
    } else {
        ""
    };
    let left_out = routes.len() - shown.len();
    let note = if left_out > 0 {
        format!(
            "<p class=\"note drawing-only\">{left_out} further away than {:.0} km left out here, but shown below.</p>",
            OVERVIEW_RADIUS_M / 1000.0
        )
    } else {
        String::new()
    };
    // The least used go first here too. A faraway route is on the map to pan to,
    // but the map starts on the others.
    let (has_map, host) = if with_map {
        let items: Vec<String> = routes
            .iter()
            .rev()
            .map(|route| {
                format!(
                    "{{\"tip\":{},\"mark\":\"{}\",\"near\":{},\"track\":{}}}",
                    json_string(&format!("{}\n{}", route.name(), route.summary())),
                    route.colour(colours),
                    near(&route),
                    positions_json(&route.newest().points, cos_lat)
                )
            })
            .collect();
        (
            " has-map",
            format!(
                "<div class=\"map-host\" data-routes=\"{}\"></div>",
                esc(&format!("[{}]", items.join(",")))
            ),
        )
    } else {
        ("", String::new())
    };
    format!(
        "<figure class=\"chart overview-map{has_map}\"><div class=\"legend\">{entries}{others}</div>{}{host}{note}</figure>\n",
        svg(OVERVIEW_SIZE, "All routes", &body)
    )
}

fn route_card(route: &Route, colours: &[(usize, &'static str)]) -> Option<String> {
    let activity = route.newest();
    let cos_lat = cos_lat_of(activity)?;
    let located = located_metres(activity, cos_lat);
    let frame = Frame::fit(&located, CARD_SIZE, PAD)?;
    let (x, y) = frame.px(located[0]);
    let name = route.name();
    let body = format!(
        "<path class=\"track {}\" d=\"{}\"/>\n<circle class=\"start\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\"/>\n{}",
        route.colour(colours),
        path(&activity.points, cos_lat, &frame),
        scale_bar(&frame, CARD_SIZE)
    );
    Some(format!(
        "<a class=\"card route-card\" href=\"#sessions\" data-route=\"{}\" \
         title=\"Name it in routes.txt: {} = ...\">{}\
         <div class=\"label\">{}</div><div class=\"detail\">{}</div>\
         <div class=\"detail\">Last run {}</div></a>\n",
        route.index,
        esc(&activity.activity_id),
        svg(CARD_SIZE, &format!("Route {name}"), &body),
        esc(name),
        esc(&route.summary()),
        activity.date
    ))
}

/// The overview and a card for each route, for the sessions of one range.
pub(crate) fn routes_section(
    sessions: &[&Session],
    colours: &[(usize, &'static str)],
    with_map: bool,
) -> String {
    let routes = routes_of(sessions);
    let no_track = sessions.iter().filter(|s| s.route.is_none()).count();
    let mut out = String::new();
    if routes.is_empty() {
        out.push_str("<p class=\"note\">No GPS tracks in this range.</p>\n");
    } else {
        out.push_str(&overview_map(&routes, colours, with_map));
        out.push_str("<div class=\"route-cards\">\n");
        for route in &routes {
            out.extend(route_card(route, colours));
        }
        out.push_str("</div>\n");
    }
    if no_track > 0 {
        out.push_str(&format!(
            "<p class=\"note\">{no_track} sessions in this range have no GPS track.</p>\n"
        ));
    }
    out
}
