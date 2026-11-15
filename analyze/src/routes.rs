//! Routes: grouping tracks that cover the same ground, describing them, and
//! naming them from routes.txt.

use std::collections::{BTreeMap, BTreeSet};

use crate::metrics::Metrics;
use crate::model::{Activity, Point, haversine};

/// Tracks are compared by the grid cells of this size they pass through.
const ROUTE_CELL_M: f64 = 100.0;
/// A track joins a route when it shares this much of its cells with it.
const ROUTE_MIN_OVERLAP: f64 = 0.6;
/// A route whose start and end are further apart than this is one-way.
const ROUTE_ONE_WAY_M: f64 = 500.0;
/// The two halves of an out-and-back share at least this much of their cells.
const ROUTE_RETRACED_OVERLAP: f64 = 0.5;
/// The route name of a session without GPS positions.
pub const NO_ROUTE: &str = "no GPS";
const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

/// The ~100 m grid cells a track passes through.
pub fn route_cells(activity: &Activity) -> BTreeSet<(i64, i64)> {
    cells_of(located(&activity.points))
}

/// The points with a GPS position.
fn located(points: &[Point]) -> impl Iterator<Item = &Point> {
    points.iter().filter(|p| p.lat != 0.0 || p.lon != 0.0)
}

fn cells_of<'p>(points: impl Iterator<Item = &'p Point>) -> BTreeSet<(i64, i64)> {
    const METRES_PER_DEGREE: f64 = 111_320.0;
    points
        .map(|p| {
            // The whole degree of latitude keeps the grid the same for every track.
            let scale = p.lat.round().to_radians().cos();
            (
                (p.lat * METRES_PER_DEGREE / ROUTE_CELL_M).floor() as i64,
                (p.lon * METRES_PER_DEGREE * scale / ROUTE_CELL_M).floor() as i64,
            )
        })
        .collect()
}

/// The compass point from one position towards another.
fn compass(from: &Point, to: &Point) -> &'static str {
    let (p1, p2) = (from.lat.to_radians(), to.lat.to_radians());
    let dl = (to.lon - from.lon).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    let degrees = y.atan2(x).to_degrees().rem_euclid(360.0);
    COMPASS[((degrees + 22.5) / 45.0) as usize % COMPASS.len()]
}

/// A name for a route from one of its tracks, like `R3 · 5.2 km loop NE`: the
/// distance, the shape, and the direction of the point farthest from the start.
pub fn describe_route(index: usize, activity: &Activity) -> String {
    let label = format!("R{}", index + 1);
    let points: Vec<&Point> = located(&activity.points).collect();
    let (Some(&start), Some(&end)) = (points.first(), points.last()) else {
        return label;
    };
    let from_start = |p: &Point| haversine(start.lat, start.lon, p.lat, p.lon);
    let Some((far, &farthest)) = points
        .iter()
        .enumerate()
        .max_by(|a, b| from_start(a.1).total_cmp(&from_start(b.1)))
    else {
        return label;
    };
    if from_start(farthest) == 0.0 {
        return label;
    }
    let out = cells_of(points[..=far].iter().copied());
    let back = cells_of(points[far..].iter().copied());
    let shape = if from_start(end) > ROUTE_ONE_WAY_M {
        "one-way"
    } else if jaccard(&out, &back) >= ROUTE_RETRACED_OVERLAP {
        "out-and-back"
    } else {
        "loop"
    };
    let km = (end.distance_m - start.distance_m) / 1000.0;
    format!("{label} · {km:.1} km {shape} {}", compass(start, farthest))
}

fn jaccard(a: &BTreeSet<(i64, i64)>, b: &BTreeSet<(i64, i64)>) -> f64 {
    let shared = a.intersection(b).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// Greedily group tracks: each joins the most similar route seen so far when the
/// overlap with that route's first track is high enough, or starts a new one.
pub fn assign_routes(tracks: &[BTreeSet<(i64, i64)>]) -> Vec<Option<usize>> {
    let mut firsts: Vec<&BTreeSet<(i64, i64)>> = Vec::new();
    tracks
        .iter()
        .map(|track| {
            if track.is_empty() {
                return None;
            }
            let best = firsts
                .iter()
                .enumerate()
                .map(|(i, first)| (i, jaccard(track, first)))
                .filter(|&(_, overlap)| overlap >= ROUTE_MIN_OVERLAP)
                .max_by(|a, b| a.1.total_cmp(&b.1));
            Some(match best {
                Some((i, _)) => i,
                None => {
                    firsts.push(track);
                    firsts.len() - 1
                }
            })
        })
        .collect()
}

/// Parse routes.txt: `<activity id> = <name>` per line, `#` comments.
pub fn parse_route_names(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut names = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        match line.split_once('=') {
            Some((id, name)) if !id.trim().is_empty() && !name.trim().is_empty() => {
                names.push((id.trim().to_string(), name.trim().to_string()));
            }
            _ => {
                return Err(format!("line {}: expected `<activity id> = <name>`", n + 1));
            }
        }
    }
    Ok(names)
}

/// What naming the routes left unresolved.
#[derive(Debug, Default, PartialEq)]
pub struct Naming {
    /// Activity ids that are not among the sessions.
    pub unmatched: Vec<String>,
    /// Later lines that named an already named route differently.
    pub conflicts: Vec<String>,
}

/// Rename each route that has one of its sessions in `names`; the first line wins.
pub fn name_routes(metrics: &mut Metrics, names: &[(String, String)]) -> Naming {
    let mut naming = Naming::default();
    let mut chosen: BTreeMap<usize, &str> = BTreeMap::new();
    for (id, name) in names {
        let Some(session) = metrics
            .sessions
            .iter()
            .find(|s| s.activity.activity_id == *id)
        else {
            naming.unmatched.push(id.clone());
            continue;
        };
        let Some(route) = session.route else {
            naming
                .conflicts
                .push(format!("{id} has no GPS track to name"));
            continue;
        };
        match chosen.get(&route) {
            Some(first) if first != name => naming
                .conflicts
                .push(format!("{id} = {name}: its route is already named {first}")),
            Some(_) => {}
            None => {
                chosen.insert(route, name);
            }
        }
    }
    for session in &mut metrics.sessions {
        if let Some(name) = session.route.and_then(|r| chosen.get(&r)) {
            session.route_name = name.to_string();
        }
    }
    naming
}
