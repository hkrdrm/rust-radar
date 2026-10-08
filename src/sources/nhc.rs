//! National Hurricane Center: active storms and their forecast geometry.

use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;

pub const SOURCE: &str = "nhc";
pub const CURRENT_STORMS_URL: &str = "https://www.nhc.noaa.gov/CurrentStorms.json";
pub const MAPSERVER_URL: &str =
    "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer";

/// Map-service layer name suffix -> our layer tag. Full names look like "AT4 Forecast Cone".
pub const STORM_LAYERS: [(&str, &str); 6] = [
    ("Forecast Cone", "cone"),
    ("Forecast Track", "forecast_track"),
    ("Forecast Points", "forecast_points"),
    ("Past Track", "past_track"),
    ("Past Points", "past_points"),
    ("Forecast Wind Radii", "wind_radii"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StormInfo {
    pub id: String,
    pub bin_number: String,
    pub name: String,
    pub classification: String,
    pub intensity_kt: u32,
    pub pressure_mb: u32,
    pub lat: f64,
    pub lon: f64,
    pub movement_dir: u32,
    pub movement_speed_mph: u32,
    pub advisory_num: String,
    pub issuance: DateTime<Utc>,
    pub public_advisory_url: Option<String>,
    pub forecast_zip_url: Option<String>,
}

pub fn layer_index_url() -> String {
    format!("{MAPSERVER_URL}?f=json")
}

pub fn layer_query_url(layer_id: u32) -> String {
    format!("{MAPSERVER_URL}/{layer_id}/query?where=1%3D1&outFields=*&f=geojson")
}

pub fn parse_current_storms(bytes: &[u8]) -> anyhow::Result<Vec<StormInfo>> {
    let root: Value = serde_json::from_slice(bytes).context("parsing CurrentStorms.json")?;
    let storms = root
        .get("activeStorms")
        .and_then(Value::as_array)
        .context("CurrentStorms.json has no activeStorms array")?;
    Ok(storms
        .iter()
        .filter_map(|raw| {
            let storm = parse_storm(raw);
            if storm.is_none() {
                warn!("skipping malformed NHC storm entry: {raw}");
            }
            storm
        })
        .collect())
}

fn str_field<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer)?.as_str()
}

/// NHC sends numbers both as JSON numbers and as strings (sometimes empty).
fn num_field(v: &Value, pointer: &str) -> Option<f64> {
    match v.pointer(pointer)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_storm(v: &Value) -> Option<StormInfo> {
    let issuance = str_field(v, "/publicAdvisory/issuance").or_else(|| str_field(v, "/lastUpdate"))?;
    let issuance = DateTime::parse_from_rfc3339(issuance).ok()?.with_timezone(&Utc);
    let whole = |pointer: &str| num_field(v, pointer).unwrap_or(0.0).max(0.0) as u32;
    Some(StormInfo {
        id: str_field(v, "/id")?.to_lowercase(),
        bin_number: str_field(v, "/binNumber")?.to_string(),
        name: str_field(v, "/name").unwrap_or("Unnamed").to_string(),
        classification: str_field(v, "/classification").unwrap_or("").to_string(),
        intensity_kt: whole("/intensity"),
        pressure_mb: whole("/pressure"),
        lat: num_field(v, "/latitudeNumeric")?,
        lon: num_field(v, "/longitudeNumeric")?,
        movement_dir: whole("/movementDir"),
        movement_speed_mph: whole("/movementSpeed"),
        advisory_num: str_field(v, "/publicAdvisory/advNum").unwrap_or("").to_string(),
        issuance,
        public_advisory_url: str_field(v, "/publicAdvisory/url").map(String::from),
        forecast_zip_url: str_field(v, "/forecastTrack/zipFile").map(String::from),
    })
}

pub fn parse_layer_index(bytes: &[u8]) -> anyhow::Result<HashMap<String, u32>> {
    let root: Value = serde_json::from_slice(bytes).context("parsing map service index")?;
    let layers = root.get("layers").and_then(Value::as_array).context("map service index has no layers")?;
    Ok(layers
        .iter()
        .filter_map(|l| {
            let name = l.get("name")?.as_str()?.to_string();
            let id = u32::try_from(l.get("id")?.as_u64()?).ok()?;
            Some((name, id))
        })
        .collect())
}

/// Unwraps a FeatureCollection and tags each feature for the frontend.
pub fn tag_features(collection: Value, storm_id: &str, layer: &str) -> anyhow::Result<Vec<Value>> {
    let Value::Object(mut object) = collection else {
        bail!("expected a GeoJSON object");
    };
    let Some(Value::Array(features)) = object.remove("features") else {
        bail!("GeoJSON has no features array: {}", Value::Object(object));
    };
    Ok(features
        .into_iter()
        .filter_map(|mut feature| {
            let props = feature.as_object_mut()?.entry("properties").or_insert_with(|| json!({}));
            if !props.is_object() {
                *props = json!({});
            }
            let props = props.as_object_mut()?;
            let wind = props.get("maxwind").or_else(|| props.get("intensity")).and_then(Value::as_f64);
            props.insert("storm_id".into(), json!(storm_id));
            props.insert("layer".into(), json!(layer));
            if let Some(wind) = wind {
                props.insert("wind_kt".into(), json!(wind));
            }
            Some(feature)
        })
        .collect())
}

/// Saffir-Simpson category from max sustained wind in knots.
pub fn category(kt: u32) -> &'static str {
    match kt {
        0..=33 => "TD",
        34..=63 => "TS",
        64..=82 => "1",
        83..=95 => "2",
        96..=112 => "3",
        113..=136 => "4",
        _ => "5",
    }
}

/// "008" / "008A" -> 8.
pub fn advisory_number(advisory: &str) -> Option<u32> {
    let digits: String = advisory.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
