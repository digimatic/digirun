//! GPX and TCX readers.

use crate::dt::DateTime;
use crate::model::{haversine, Lap, Point};
use crate::xml::Node;

fn num(value: &str) -> Option<f64> {
    value.parse::<f64>().ok()
}

/// Trackpoints plus the track name. Distance is derived from positions.
pub fn read_gpx(root: &Node) -> (Vec<Point>, String) {
    let trk = match root.find("trk") {
        Some(trk) => trk,
        None => return (Vec::new(), root.text("name")),
    };
    let name = trk.text("name");

    let mut points: Vec<Point> = Vec::new();
    let mut cumulative = 0.0;
    let mut prev: Option<(f64, f64)> = None;

    for seg in trk.children_named("trkseg") {
        for pt in seg.children_named("trkpt") {
            let lat: f64 = pt.attr("lat").unwrap_or("0").parse().unwrap_or(0.0);
            let lon: f64 = pt.attr("lon").unwrap_or("0").parse().unwrap_or(0.0);
            let time = match DateTime::from_iso(&pt.text("time")) {
                Some(time) => time,
                None => continue,
            };
            if let Some((plat, plon)) = prev {
                cumulative += haversine(plat, plon, lat, lon);
            }
            let tpx = pt
                .find("extensions")
                .and_then(|ext| ext.find("TrackPointExtension"));
            let point = Point {
                time,
                lat,
                lon,
                distance_m: cumulative,
                cadence: tpx.and_then(|tpx| num(&tpx.text("cad"))),
                hr: tpx.and_then(|tpx| num(&tpx.text("hr"))),
                altitude: num(&pt.text("ele")),
            };
            points.push(point);
            prev = Some((lat, lon));
        }
    }

    (points, name)
}

/// Trackpoints and laps. DistanceMeters is cumulative within each lap.
pub fn read_tcx(root: &Node) -> (Vec<Point>, Vec<Lap>) {
    let activity = match root.find("Activities").and_then(|a| a.find("Activity")) {
        Some(activity) => activity,
        None => return (Vec::new(), Vec::new()),
    };

    let mut points: Vec<Point> = Vec::new();
    let mut laps: Vec<Lap> = Vec::new();

    for lap in activity.children_named("Lap") {
        let start_index = points.len();
        let duration = num(&lap.text("TotalTimeSeconds")).unwrap_or(0.0);
        let distance = num(&lap.text("DistanceMeters")).unwrap_or(0.0);

        for tp in lap
            .find("Track")
            .into_iter()
            .flat_map(|t| t.children_named("Trackpoint"))
        {
            let time = match DateTime::from_iso(&tp.text("Time")) {
                Some(time) => time,
                None => continue,
            };
            // Position is optional: some exports have no GPS, so DistanceMeters
            // (cumulative within the lap) is the only distance available.
            let pos = tp.find("Position");
            let lat: f64 = pos
                .map(|p| p.text("LatitudeDegrees"))
                .unwrap_or_default()
                .parse()
                .unwrap_or(0.0);
            let lon: f64 = pos
                .map(|p| p.text("LongitudeDegrees"))
                .unwrap_or_default()
                .parse()
                .unwrap_or(0.0);
            let cadence = match tp.find("Extensions") {
                Some(ext) => match ext.find("TPX") {
                    Some(tpx) => num(&tpx.text("RunCadence")),
                    None => None,
                },
                None => None,
            };
            points.push(Point {
                time,
                lat,
                lon,
                distance_m: num(&tp.text("DistanceMeters")).unwrap_or(0.0),
                cadence,
                hr: tp
                    .find("HeartRateBpm")
                    .and_then(|hr| num(&hr.text("Value"))),
                altitude: num(&tp.text("AltitudeMeters")),
            });
        }

        if points.len() > start_index {
            laps.push(Lap {
                start_index,
                end_index: points.len() - 1,
                duration_s: duration,
                distance_m: distance,
            });
        }
    }

    (points, laps)
}
