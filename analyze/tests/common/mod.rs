//! Fixtures shared by the integration tests, which mirror `tests/test_analyze.py`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use garmin_analyze::args::Args;
use garmin_analyze::dt::{Date, DateTime};
use garmin_analyze::metrics::build;
use garmin_analyze::model::{Activity, Lap, Point, haversine};
use garmin_analyze::routes::{name_routes, parse_route_names};
use garmin_analyze::serve::Request;

pub const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx xmlns="http://www.topografix.com/GPX/1/1"
     xmlns:ns3="http://www.garmin.com/xmlschemas/TrackPointExtension/v1" version="1.1">
  <trk>
    <name>Test walk-run</name>
    <trkseg>
      <trkpt lat="59.33258" lon="18.06490">
        <time>2026-08-07T14:26:09.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>50</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
      <trkpt lat="59.33266" lon="18.06490">
        <time>2026-08-07T14:26:19.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>52</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
      <trkpt lat="59.33274" lon="18.06490">
        <time>2026-08-07T14:26:29.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>49</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
      <trkpt lat="59.33282" lon="18.06490">
        <time>2026-08-07T14:26:39.000Z</time>
        <extensions><ns3:TrackPointExtension><ns3:cad>80</ns3:cad></ns3:TrackPointExtension></extensions>
      </trkpt>
    </trkseg>
  </trk>
</gpx>
"#;

/// A timestamp with the UTC offset it carries, as in the fixture.
pub fn time(hour: u32, minute: u32, second: u32) -> DateTime {
    DateTime {
        date: Date {
            year: 2026,
            month: 8,
            day: 7,
        },
        hour,
        minute,
        second,
        micro: 0,
        offset: 0,
    }
}

/// A point `seconds` after 14:26:00 on 2026-08-07, normalised into a valid time.
pub fn point(seconds: i64, distance_m: f64, cadence: Option<f64>) -> Point {
    let total = 14 * 3600 + 26 * 60 + seconds;
    Point {
        time: time(
            (total / 3600) as u32,
            ((total % 3600) / 60) as u32,
            (total % 60) as u32,
        ),
        lat: 0.0,
        lon: 0.0,
        distance_m,
        cadence,
        hr: None,
        altitude: None,
    }
}

pub fn activity(points: &[Point]) -> Activity {
    Activity {
        activity_id: "1".to_string(),
        name: "Test".to_string(),
        date: points[0].time.date,
        points: points.to_vec(),
        laps: Vec::new(),
        r#type: String::new(),
    }
}

/// A unique directory under the system temp dir, cleaned up by the OS.
pub fn temp_dir(label: &str) -> PathBuf {
    let unique = std::process::id().to_string() + "-" + label;
    let path = std::env::temp_dir()
        .join("garmin-analyze-tests")
        .join(unique);
    std::fs::create_dir_all(&path).expect("could not create the test directory");
    path
}

pub fn first_line(path: &Path) -> String {
    std::fs::read_to_string(path)
        .expect("could not read the CSV")
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// A 1 Hz track from `(seconds, cadence, metres per second)` stretches.
pub fn track(stretches: &[(i64, f64, f64)]) -> Vec<Point> {
    let mut points = vec![point(0, 0.0, None)];
    let (mut t, mut distance) = (0, 0.0);
    for &(seconds, cadence, speed) in stretches {
        for _ in 0..seconds {
            t += 1;
            distance += speed;
            points.push(point(t, distance, Some(cadence)));
        }
    }
    points
}

pub fn with_one_lap(points: &[Point]) -> Activity {
    let mut act = activity(points);
    act.laps = vec![Lap {
        start_index: 0,
        end_index: points.len() - 1,
        duration_s: (points.len() - 1) as f64,
        distance_m: points[points.len() - 1].distance_m,
    }];
    act
}

pub fn date(year: i32, month: u32, day: u32) -> Date {
    Date { year, month, day }
}

/// An activity on `on`, with HR from `hr(second)`.
pub fn session_on(
    on: Date,
    id: &str,
    stretches: &[(i64, f64, f64)],
    hr: fn(usize) -> f64,
) -> Activity {
    let mut points = track(stretches);
    for (i, p) in points.iter_mut().enumerate() {
        p.hr = Some(hr(i));
    }
    Activity {
        activity_id: id.to_string(),
        date: on,
        ..activity(&points)
    }
}

pub fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

/// A track through these offsets in metres east and north, a point every ~20 m.
pub fn shaped(corners: &[(f64, f64)]) -> Activity {
    let (lat0, lon0) = (57.7, 12.0);
    let metres_per_degree = 111_320.0;
    let mut offsets = vec![corners[0]];
    for pair in corners.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let steps = ((x1 - x0).hypot(y1 - y0) / 20.0).ceil() as usize;
        for k in 1..=steps {
            let f = k as f64 / steps as f64;
            offsets.push((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f));
        }
    }
    let mut points: Vec<Point> = Vec::new();
    for (i, (x, y)) in offsets.into_iter().enumerate() {
        let mut p = point(i as i64 * 5, 0.0, Some(80.0));
        p.lat = lat0 + y / metres_per_degree;
        p.lon = lon0 + x / (metres_per_degree * lat0.to_radians().cos());
        if let Some(last) = points.last() {
            p.distance_m = last.distance_m + haversine(last.lat, last.lon, p.lat, p.lon);
        }
        points.push(p);
    }
    activity(&points)
}

/// Two sessions on a home route, one far away, one without GPS; home named "Home".
pub fn named_metrics(runs: &[Activity; 4]) -> garmin_analyze::metrics::Metrics<'_> {
    let refs: Vec<&Activity> = runs.iter().collect();
    let mut metrics = build(&refs, &Args::default());
    name_routes(&mut metrics, &parse_route_names("1 = Home").unwrap());
    metrics
}

pub fn four_runs() -> [Activity; 4] {
    [
        run_from("1", 3, 57.74, 12.12),
        run_from("3", 5, 40.0, -3.7),
        run_from("2", 20, 57.74, 12.12),
        run_on("4", "Run", 21),
    ]
}

pub fn html_args(hr_max: Option<i64>) -> Args {
    Args {
        hr_max,
        ..Args::default()
    }
}

/// Eight minutes of running with HR on a given day of August 2026.
pub fn run_on(id: &str, name: &str, day: u32) -> Activity {
    let mut points = track(&[(480, 80.0, 3.0)]);
    for point in &mut points {
        point.time.date.day = day;
        point.hr = Some(150.0);
    }
    let mut activity = activity(&points);
    activity.activity_id = id.to_string();
    activity.name = name.to_string();
    activity
}

pub fn august(day: u32) -> Date {
    Date {
        year: 2026,
        month: 8,
        day,
    }
}

/// `run_on`, heading north-east from a starting position.
pub fn run_from(id: &str, day: u32, lat: f64, lon: f64) -> Activity {
    let mut run = run_on(id, "Run", day);
    for (i, point) in run.points.iter_mut().enumerate() {
        point.lat = lat + i as f64 * 0.00002;
        point.lon = lon + i as f64 * 0.00003;
    }
    run
}

pub fn rename_request(body: &str) -> Request {
    Request {
        method: "POST".to_string(),
        path: "/route-name".to_string(),
        headers: vec![
            ("host".to_string(), "127.0.0.1:8765".to_string()),
            ("x-digirun".to_string(), "rename".to_string()),
        ],
        body: body.as_bytes().to_vec(),
    }
}
