//! HTTP API, radar tiles and the static frontend.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, SecondsFormat, Utc};
use lru::LruCache;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::ServeDir;

use crate::mrms_archive::MrmsArchive;
use crate::nhc_archive::NhcArchive;
use crate::sources::nhc::category;
use crate::status::Status;
use crate::tiles;

const TILE_CACHE_TILES: usize = 2048;
type TileKey = (i64, u8, u32, u32);

pub struct AppState {
    pub mrms: Arc<MrmsArchive>,
    pub nhc: Arc<NhcArchive>,
    pub status: Status,
    pub basemap_style: String,
    tile_cache: Mutex<LruCache<TileKey, Bytes>>,
}

impl AppState {
    pub fn new(mrms: Arc<MrmsArchive>, nhc: Arc<NhcArchive>, status: Status, basemap_style: String) -> AppState {
        let capacity = NonZeroUsize::new(TILE_CACHE_TILES).expect("non-zero");
        AppState { mrms, nhc, status, basemap_style, tile_cache: Mutex::new(LruCache::new(capacity)) }
    }
}

pub fn router(state: Arc<AppState>, web_dir: &Path) -> Router {
    Router::new()
        .route("/api/config", get(get_config))
        .route("/api/frames", get(get_frames))
        .route("/api/status", get(get_status))
        .route("/api/storms", get(get_storms))
        .route("/api/storms/{id}", get(get_storm))
        .route("/tiles/{time}/{z}/{x}/{y}", get(get_tile))
        .fallback_service(ServeDir::new(web_dir))
        .with_state(state)
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Internal(e) => {
                tracing::error!("{e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        (code, Json(json!({ "error": message }))).into_response()
    }
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(s: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| ApiError::BadRequest(format!("invalid time {s:?}; use RFC 3339, e.g. 2026-10-08T17:00:00Z")))
}

#[derive(Deserialize)]
struct TimeQuery {
    time: Option<String>,
}

impl TimeQuery {
    fn resolve(&self) -> Result<DateTime<Utc>, ApiError> {
        self.time.as_deref().map_or_else(|| Ok(Utc::now()), parse_time)
    }
}

async fn get_config(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "basemap_style": st.basemap_style }))
}

async fn get_frames(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let frames = st.mrms.frames()?;
    let latest = frames.last().copied().map(rfc3339);
    let frames: Vec<String> = frames.into_iter().map(rfc3339).collect();
    Ok(Json(json!({ "frames": frames, "latest": latest })))
}

async fn get_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "now": rfc3339(Utc::now()), "sources": st.status.snapshot() }))
}

async fn get_storms(State(st): State<Arc<AppState>>, Query(q): Query<TimeQuery>) -> Result<Json<Value>, ApiError> {
    let t = q.resolve()?;
    let mut features = Vec::new();
    for snapshot in st.nhc.at_time(t)? {
        let s = &snapshot.storm;
        features.push(json!({
            "type": "Feature",
            "geometry": { "type": "Point", "coordinates": [s.lon, s.lat] },
            "properties": {
                "storm_id": s.id,
                "layer": "center",
                "name": s.name,
                "classification": s.classification,
                "category": category(s.intensity_kt),
                "wind_kt": s.intensity_kt,
                "pressure_mb": s.pressure_mb,
                "advisory_num": s.advisory_num,
                "issuance": rfc3339(s.issuance),
            }
        }));
        features.extend(snapshot.features);
    }
    Ok(Json(json!({ "type": "FeatureCollection", "features": features })))
}

async fn get_storm(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<TimeQuery>,
) -> Result<Json<Value>, ApiError> {
    let t = q.resolve()?;
    let snapshot = st
        .nhc
        .get(&id.to_lowercase(), t)?
        .ok_or_else(|| ApiError::NotFound(format!("no advisory for storm {id:?} at {}", rfc3339(t))))?;
    let category = category(snapshot.storm.intensity_kt);
    Ok(Json(json!({ "storm": snapshot.storm, "category": category, "geometry_source": snapshot.geometry_source })))
}

async fn get_tile(
    State(st): State<Arc<AppState>>,
    UrlPath((time, z, x, y)): UrlPath<(String, u8, u32, String)>,
) -> Result<Response, ApiError> {
    let y: u32 = y
        .strip_suffix(".png")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ApiError::BadRequest(format!("bad tile row {y:?}; expected e.g. 12.png")))?;
    if !tiles::valid_tile(z, x, y) {
        return Err(ApiError::BadRequest(format!("tile {z}/{x}/{y} is out of range (max zoom {})", tiles::MAX_ZOOM)));
    }
    let is_latest = time == "latest";
    let t = if is_latest {
        st.mrms.latest()?.ok_or_else(|| ApiError::NotFound("no radar frames yet".into()))?
    } else {
        let t = parse_time(&time)?;
        if !st.mrms.contains(t) {
            return Err(ApiError::NotFound(format!("no radar frame at {}", rfc3339(t))));
        }
        t
    };

    if st.mrms.known_bounds().is_some_and(|b| !tiles::tile_overlaps(b, z, x, y)) {
        let cache_control = if is_latest { "no-cache" } else { "public, max-age=86400" };
        let png = Bytes::from_static(tiles::empty_tile_png());
        return Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, cache_control)], png).into_response());
    }

    let key = (t.timestamp(), z, x, y);
    let cached = st.tile_cache.lock().unwrap().get(&key).cloned();
    let png = match cached {
        Some(png) => png,
        None => {
            let state = Arc::clone(&st);
            let rendered = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let grid = state.mrms.load(t)?;
                Ok(tiles::render_tile(&grid, z, x, y))
            })
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("tile render task failed: {e}")))??;
            let png = Bytes::from(rendered);
            st.tile_cache.lock().unwrap().put(key, png.clone());
            png
        }
    };
    let cache_control = if is_latest { "no-cache" } else { "public, max-age=86400" };
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, cache_control)], png).into_response())
}
