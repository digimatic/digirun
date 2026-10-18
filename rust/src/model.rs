//! The data model: points, laps, activities, segments and per-class stats.

use crate::dt::{Date, DateTime};

#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub time: DateTime,
    pub lat: f64,
    pub lon: f64,
    /// Cumulative distance in metres (from positions in GPX, from DistanceMeters in TCX).
    pub distance_m: f64,
    pub cadence: Option<f64>,
    pub hr: Option<f64>,
    pub altitude: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lap {
    pub start_index: usize,
    pub end_index: usize,
    pub duration_s: f64,
    pub distance_m: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub activity_id: String,
    pub name: String,
    pub date: Date,
    pub points: Vec<Point>,
    pub laps: Vec<Lap>,
    /// Activity type key, read from summary.csv.
    pub r#type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub activity_id: String,
    pub date: Date,
    pub time: DateTime,
    pub duration_s: f64,
    pub distance_m: f64,
    pub pace_s_per_km: f64,
    pub cadence: Option<f64>,
    pub class: &'static str,
}

/// Mean/stdev of pace, plus cadence where the segments have it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub segments: usize,
    pub time_s: f64,
    pub distance_km: f64,
    pub mean: f64,
    pub stdev: f64,
    pub min: f64,
    pub max: f64,
    pub cad_mean: Option<f64>,
    pub cad_stdev: Option<f64>,
}

/// Great-circle distance in metres.
pub fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6371000.0;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}
