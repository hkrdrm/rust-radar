//! Fallback geometry: NHC's per-advisory 5-day shapefile zip (cone, track, points).

use std::io::{Cursor, Read};

use anyhow::{bail, Context};
use serde_json::{json, Map, Value};
use shapefile::dbase::{FieldValue, Record};
use shapefile::{PolygonRing, Shape};

use crate::sources::nhc::tag_features;

/// Shapefile name suffix inside the zip -> our layer tag.
const ZIP_LAYERS: [(&str, &str); 3] = [
    ("_5day_pgn", "cone"),
    ("_5day_lin", "forecast_track"),
    ("_5day_pts", "forecast_points"),
];

pub fn features_from_zip(bytes: &[u8], storm_id: &str) -> anyhow::Result<Vec<Value>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("opening advisory zip")?;
    let mut out = Vec::new();
    for (suffix, layer) in ZIP_LAYERS {
        let shp = read_entry(&mut zip, &format!("{suffix}.shp"))?;
        let dbf = read_entry(&mut zip, &format!("{suffix}.dbf"))?;
        let shapes = shapefile::ShapeReader::new(Cursor::new(shp)).with_context(|| format!("reading {suffix}.shp"))?;
        let records = shapefile::dbase::Reader::new(Cursor::new(dbf)).with_context(|| format!("reading {suffix}.dbf"))?;
        let mut reader = shapefile::Reader::new(shapes, records);
        let mut features = Vec::new();
        for item in reader.iter_shapes_and_records() {
            let (shape, record) = item.with_context(|| format!("reading a {suffix} record"))?;
            features.push(json!({
                "type": "Feature",
                "geometry": shape_to_geojson(&shape)?,
                "properties": record_to_json(record),
            }));
        }
        out.extend(tag_features(json!({"type": "FeatureCollection", "features": features}), storm_id, layer)?);
    }
    if !out.iter().any(|f| f["properties"]["layer"] == "cone") {
        bail!("advisory zip has no forecast cone");
    }
    Ok(out)
}

fn read_entry(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, suffix: &str) -> anyhow::Result<Vec<u8>> {
    let name = zip
        .file_names()
        .find(|n| n.ends_with(suffix))
        .map(str::to_owned)
        .with_context(|| format!("advisory zip has no *{suffix}"))?;
    let mut buf = Vec::new();
    zip.by_name(&name)?.read_to_end(&mut buf)?;
    Ok(buf)
}

trait Xy {
    fn xy(&self) -> [f64; 2];
}
impl Xy for shapefile::Point {
    fn xy(&self) -> [f64; 2] { [self.x, self.y] }
}
impl Xy for shapefile::PointM {
    fn xy(&self) -> [f64; 2] { [self.x, self.y] }
}
impl Xy for shapefile::PointZ {
    fn xy(&self) -> [f64; 2] { [self.x, self.y] }
}

fn coords<P: Xy>(points: &[P]) -> Vec<[f64; 2]> {
    points.iter().map(Xy::xy).collect()
}

fn lines<P: Xy>(parts: &[Vec<P>]) -> Value {
    if parts.len() == 1 {
        json!({"type": "LineString", "coordinates": coords(&parts[0])})
    } else {
        json!({"type": "MultiLineString", "coordinates": parts.iter().map(|p| coords(p)).collect::<Vec<_>>()})
    }
}

/// Each outer ring starts a polygon; inner rings are holes in the preceding polygon.
fn polygons<P: Xy>(rings: &[PolygonRing<P>]) -> Value {
    let mut polygons: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
    for ring in rings {
        match ring {
            PolygonRing::Outer(points) => polygons.push(vec![coords(points)]),
            PolygonRing::Inner(points) => match polygons.last_mut() {
                Some(polygon) => polygon.push(coords(points)),
                None => polygons.push(vec![coords(points)]),
            },
        }
    }
    if polygons.len() == 1 {
        json!({"type": "Polygon", "coordinates": polygons.remove(0)})
    } else {
        json!({"type": "MultiPolygon", "coordinates": polygons})
    }
}

fn shape_to_geojson(shape: &Shape) -> anyhow::Result<Value> {
    Ok(match shape {
        Shape::Point(p) => json!({"type": "Point", "coordinates": p.xy()}),
        Shape::PointM(p) => json!({"type": "Point", "coordinates": p.xy()}),
        Shape::PointZ(p) => json!({"type": "Point", "coordinates": p.xy()}),
        Shape::Polyline(l) => lines(l.parts()),
        Shape::PolylineM(l) => lines(l.parts()),
        Shape::PolylineZ(l) => lines(l.parts()),
        Shape::Polygon(p) => polygons(p.rings()),
        Shape::PolygonM(p) => polygons(p.rings()),
        Shape::PolygonZ(p) => polygons(p.rings()),
        _ => bail!("unsupported shapefile geometry"),
    })
}

/// DBF attributes as JSON with lower-cased keys, matching the map service's field names.
fn record_to_json(record: Record) -> Value {
    let mut props = Map::new();
    for (name, value) in record {
        let value = match value {
            FieldValue::Character(Some(s)) => json!(s.trim()),
            FieldValue::Numeric(Some(n)) => json!(n),
            FieldValue::Float(Some(f)) => json!(f),
            FieldValue::Double(d) => json!(d),
            FieldValue::Integer(i) => json!(i),
            FieldValue::Logical(Some(b)) => json!(b),
            _ => Value::Null,
        };
        props.insert(name.to_lowercase(), value);
    }
    Value::Object(props)
}
