use chrono::{TimeZone, Utc};
use serde_json::json;
use rust_radar::sources::nhc::*;

const CURRENT_STORMS: &str = r#"{"activeStorms":[
 {"id":"al092026","binNumber":"AT4","name":"Isaias","classification":"HU","intensity":"75","pressure":"975",
  "latitudeNumeric":23.7,"longitudeNumeric":-90.2,"movementDir":60,"movementSpeed":10,
  "lastUpdate":"2026-10-08T15:00:00.000Z",
  "publicAdvisory":{"advNum":"008","issuance":"2026-10-08T15:00:00.000Z","url":"https://www.nhc.noaa.gov/text/MIATCPAT4.shtml"},
  "forecastTrack":{"advNum":"008","zipFile":"https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip"}},
 {"id":"EP182026","binNumber":"EP3","name":"Rosa","classification":"PTC","intensity":"","pressure":"1006",
  "latitudeNumeric":"19.1","longitudeNumeric":-121.4,"movementDir":270,"movementSpeed":"5",
  "lastUpdate":"2026-10-08T15:00:00.000Z"},
 {"id":"ep202026","name":"Broken"}
]}"#;

#[test]
fn parses_storms_leniently_and_skips_malformed() {
    let storms = parse_current_storms(CURRENT_STORMS.as_bytes()).unwrap();
    assert_eq!(storms.len(), 2);

    let isaias = &storms[0];
    assert_eq!(isaias.id, "al092026");
    assert_eq!(isaias.bin_number, "AT4");
    assert_eq!(isaias.intensity_kt, 75);
    assert_eq!(isaias.pressure_mb, 975);
    assert_eq!((isaias.lat, isaias.lon), (23.7, -90.2));
    assert_eq!(isaias.advisory_num, "008");
    assert_eq!(isaias.issuance, Utc.with_ymd_and_hms(2026, 10, 8, 15, 0, 0).unwrap());
    assert_eq!(isaias.forecast_zip_url.as_deref(), Some("https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip"));

    let rosa = &storms[1];
    assert_eq!(rosa.id, "ep182026", "ids are lower-cased");
    assert_eq!(rosa.intensity_kt, 0, "empty intensity becomes 0");
    assert_eq!(rosa.lat, 19.1, "numeric strings are accepted");
    assert_eq!(rosa.movement_speed_mph, 5);
    assert_eq!(rosa.advisory_num, "");
    assert_eq!(rosa.public_advisory_url, None);
}

#[test]
fn empty_and_invalid_feeds() {
    assert!(parse_current_storms(br#"{"activeStorms":[]}"#).unwrap().is_empty());
    assert!(parse_current_storms(b"{}").is_err());
    assert!(parse_current_storms(b"<html>").is_err());
}

#[test]
fn saffir_simpson_categories() {
    let cases = [(33, "TD"), (34, "TS"), (63, "TS"), (64, "1"), (82, "1"), (83, "2"), (95, "2"),
                 (96, "3"), (112, "3"), (113, "4"), (136, "4"), (137, "5")];
    for (kt, expected) in cases {
        assert_eq!(category(kt), expected, "{kt} kt");
    }
}

#[test]
fn advisory_numbers() {
    assert_eq!(advisory_number("008"), Some(8));
    assert_eq!(advisory_number("8"), Some(8));
    assert_eq!(advisory_number("008A"), Some(8));
    assert_eq!(advisory_number(""), None);
}

#[test]
fn layer_index_maps_names_to_ids() {
    let index = parse_layer_index(br#"{"layers":[{"id":86,"name":"AT4 Forecast Cone","parentLayerId":81},{"id":4,"name":"AT1","parentLayerId":-1}]}"#).unwrap();
    assert_eq!(index.get("AT4 Forecast Cone"), Some(&86));
    assert_eq!(index.get("AT1"), Some(&4));
    assert!(parse_layer_index(b"{}").is_err());
    assert_eq!(
        layer_query_url(86),
        "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer/86/query?where=1%3D1&outFields=*&f=geojson"
    );
}

#[test]
fn tag_features_adds_ids_and_wind() {
    let fc = json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "geometry": null, "properties": {"maxwind": 75}},
        {"type": "Feature", "geometry": null, "properties": null},
        {"type": "Feature", "geometry": null, "properties": {"intensity": 15}},
        "not a feature"
    ]});
    let tagged = tag_features(fc, "al092026", "forecast_points").unwrap();
    assert_eq!(tagged.len(), 3);
    for f in &tagged {
        assert_eq!(f["properties"]["storm_id"], "al092026");
        assert_eq!(f["properties"]["layer"], "forecast_points");
    }
    assert_eq!(tagged[0]["properties"]["wind_kt"], 75.0);
    assert!(tagged[1]["properties"].get("wind_kt").is_none());
    assert_eq!(tagged[2]["properties"]["wind_kt"], 15.0);

    // ArcGIS reports query errors as HTTP 200 with an "error" object.
    assert!(tag_features(json!({"error": {"code": 400}}), "x", "cone").is_err());
}
