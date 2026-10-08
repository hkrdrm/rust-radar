mod common;

use rust_radar::sources::nhc_zip::features_from_zip;
use serde_json::Value;

fn of_layer<'a>(features: &'a [Value], layer: &str) -> Vec<&'a Value> {
    features.iter().filter(|f| f["properties"]["layer"] == layer).collect()
}

#[test]
fn reads_cone_track_and_points() {
    let features = features_from_zip(&common::nhc_zip_fixture(), "al092026").unwrap();
    assert!(features.iter().all(|f| f["properties"]["storm_id"] == "al092026"));

    let cone = of_layer(&features, "cone");
    assert!(!cone.is_empty());
    assert!(matches!(cone[0]["geometry"]["type"].as_str(), Some("Polygon" | "MultiPolygon")));

    let track = of_layer(&features, "forecast_track");
    assert!(!track.is_empty());
    assert!(matches!(track[0]["geometry"]["type"].as_str(), Some("LineString" | "MultiLineString")));

    let points = of_layer(&features, "forecast_points");
    assert!(points.len() >= 3);
    assert_eq!(points[0]["geometry"]["type"], "Point");
    assert!(points[0]["properties"]["wind_kt"].is_number(), "MAXWIND is lower-cased and mapped");
}

#[test]
fn rejects_non_zip() {
    assert!(features_from_zip(b"not a zip", "al092026").is_err());
}
