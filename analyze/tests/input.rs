//! Reading GPX, TCX, summary.csv and dates.

mod common;

use std::collections::BTreeMap;

use garmin_analyze::load::{activity_types, load_activities};
use garmin_analyze::parse::{read_gpx, read_tcx};
use garmin_analyze::xml;

use common::*;

#[test]
fn read_gpx_reads_points_and_cadence() {
    let dir = temp_dir("read_gpx");
    let path = dir.join("2026-08-07_1426_test_1.gpx");
    std::fs::write(&path, FIXTURE).expect("could not write the fixture");

    let content = std::fs::read_to_string(&path).expect("could not read the fixture");
    let root = xml::parse(&content).expect("could not parse the fixture");
    let (points, name) = read_gpx(&root);

    assert_eq!(name, "Test walk-run");
    assert_eq!(points.len(), 4);
    assert_eq!(
        points.iter().map(|p| p.cadence).collect::<Vec<_>>(),
        vec![Some(50.0), Some(52.0), Some(49.0), Some(80.0)]
    );
    // 0.00008 deg of latitude is about 8.9 m per step, so distance accumulates.
    assert_eq!(points[0].distance_m, 0.0);
    assert!(points[1].distance_m > 8.0);
    assert!(points[3].distance_m > points[2].distance_m);
}

#[test]
fn activity_types_from_summary_csv() {
    let dir = temp_dir("summary");
    std::fs::write(
        dir.join("summary.csv"),
        "date,activity_id,name,type\n2026-08-07,1,Run,running\n2026-08-07,2,Ride,cycling\n",
    )
    .expect("could not write summary.csv");

    let mut expected = BTreeMap::new();
    expected.insert("1".to_string(), "running".to_string());
    expected.insert("2".to_string(), "cycling".to_string());
    assert_eq!(activity_types(&dir), expected);
    assert_eq!(activity_types(&dir.join("missing")), BTreeMap::new());
}

#[test]
fn load_activities_is_the_same_with_and_without_the_pool() {
    // enough files to cross MIN_PARALLEL_FILES, so the pool path is exercised
    let dir = temp_dir("pool");
    for i in 0..10 {
        std::fs::write(dir.join(format!("2026-08-07_{}.gpx", 1000 + i)), FIXTURE)
            .expect("could not write the fixture");
    }

    let serial = load_activities(&dir, 1);
    let parallel = load_activities(&dir, 4);

    assert_eq!(parallel.activities, serial.activities);
    assert_eq!(parallel.skipped, serial.skipped);
    assert_eq!(serial.activities.len(), 10);
}

#[test]
fn read_tcx_reads_hr_and_altitude() {
    let tcx = r#"<?xml version="1.0" encoding="UTF-8"?>
<TrainingCenterDatabase xmlns="http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2"
    xmlns:ns3="http://www.garmin.com/xmlschemas/ActivityExtension/v2">
  <Activities><Activity Sport="Running"><Lap StartTime="2026-08-07T14:26:00.000Z">
    <TotalTimeSeconds>1.0</TotalTimeSeconds><DistanceMeters>3.0</DistanceMeters>
    <Track>
      <Trackpoint><Time>2026-08-07T14:26:00.000Z</Time><AltitudeMeters>128.5</AltitudeMeters>
        <DistanceMeters>0.0</DistanceMeters><HeartRateBpm><Value>94</Value></HeartRateBpm>
        <Extensions><ns3:TPX><ns3:RunCadence>54</ns3:RunCadence></ns3:TPX></Extensions></Trackpoint>
      <Trackpoint><Time>2026-08-07T14:26:01.000Z</Time>
        <DistanceMeters>3.0</DistanceMeters></Trackpoint>
    </Track>
  </Lap></Activity></Activities>
</TrainingCenterDatabase>"#;
    let root = xml::parse(tcx).expect("could not parse the fixture");
    let (points, laps) = read_tcx(&root);

    assert_eq!(laps.len(), 1);
    assert_eq!(
        (points[0].hr, points[0].altitude, points[0].cadence),
        (Some(94.0), Some(128.5), Some(54.0))
    );
    assert_eq!((points[1].hr, points[1].altitude), (None, None));
}

#[test]
fn read_gpx_reads_hr_and_elevation() {
    let gpx = FIXTURE.replace(
        "<ns3:cad>50</ns3:cad>",
        "<ns3:hr>101</ns3:hr><ns3:cad>50</ns3:cad>",
    );
    let gpx = gpx.replacen("<time>", "<ele>12.5</ele><time>", 1);
    let root = xml::parse(&gpx).expect("could not parse the fixture");
    let (points, _) = read_gpx(&root);

    assert_eq!(
        (points[0].hr, points[0].altitude),
        (Some(101.0), Some(12.5))
    );
    assert_eq!((points[1].hr, points[1].altitude), (None, None));
}

#[test]
fn isocalendar_follows_the_thursday() {
    assert_eq!(date(2026, 11, 3).isocalendar(), (2026, 45));
    assert_eq!(date(2025, 12, 29).isocalendar(), (2026, 1));
    assert_eq!(date(2026, 12, 31).isocalendar(), (2026, 53));
    assert_eq!(date(2021, 1, 3).isocalendar(), (2020, 53));
    assert_eq!(date(2026, 11, 3).monday(), date(2026, 11, 2));
}
