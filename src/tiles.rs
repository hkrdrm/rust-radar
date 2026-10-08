//! Web Mercator tile maths and radar tile rendering.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::grid::Grid;
use crate::palette::dbz_to_rgba;

pub const TILE_SIZE: u32 = 256;
pub const MAX_ZOOM: u8 = 12;

pub fn valid_tile(z: u8, x: u32, y: u32) -> bool {
    z <= MAX_ZOOM && u64::from(x) < (1u64 << z) && u64::from(y) < (1u64 << z)
}

/// Lat/lon of a point in tile (z, x, y); `fx`/`fy` run 0..=1 from the tile's top-left corner.
pub fn tile_point_to_latlon(z: u8, x: u32, y: u32, fx: f64, fy: f64) -> (f64, f64) {
    let n = f64::from(1u32 << z);
    let lon = (f64::from(x) + fx) / n * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * (f64::from(y) + fy) / n)).sinh().atan().to_degrees();
    (lat, lon)
}

pub fn latlon_to_tile(z: u8, lat: f64, lon: f64) -> (u32, u32) {
    let n = f64::from(1u32 << z);
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let lat_r = lat.to_radians();
    let y = ((1.0 - (lat_r.tan() + 1.0 / lat_r.cos()).ln() / PI) / 2.0 * n).floor();
    let max = n - 1.0;
    (x.clamp(0.0, max) as u32, y.clamp(0.0, max) as u32)
}

/// (south, west, north, east) of a tile, degrees.
pub fn tile_bounds(z: u8, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let (north, west) = tile_point_to_latlon(z, x, y, 0.0, 0.0);
    let (south, east) = tile_point_to_latlon(z, x, y, 1.0, 1.0);
    (south, west, north, east)
}

/// Whether tile (z, x, y) overlaps `bounds` given as (south, west, north, east).
pub fn tile_overlaps(bounds: (f64, f64, f64, f64), z: u8, x: u32, y: u32) -> bool {
    let (s, w, n, e) = tile_bounds(z, x, y);
    let (gs, gw, gn, ge) = bounds;
    s < gn && n > gs && w < ge && e > gw
}

pub fn render_tile(grid: &Grid, z: u8, x: u32, y: u32) -> Vec<u8> {
    if !tile_overlaps(grid.bounds(), z, x, y) {
        return empty_tile_png().to_vec();
    }
    let size = TILE_SIZE as usize;
    // Latitude depends only on the pixel row and longitude only on the column.
    let centre = |p: usize| (p as f64 + 0.5) / size as f64;
    let lons: Vec<f64> = (0..size).map(|px| tile_point_to_latlon(z, x, y, centre(px), 0.0).1).collect();
    let lats: Vec<f64> = (0..size).map(|py| tile_point_to_latlon(z, x, y, 0.0, centre(py)).0).collect();
    let mut rgba = vec![0u8; size * size * 4];
    for (py, &lat) in lats.iter().enumerate() {
        for (px, &lon) in lons.iter().enumerate() {
            if let Some(dbz) = grid.sample(lat, lon) {
                let i = (py * size + px) * 4;
                rgba[i..i + 4].copy_from_slice(&dbz_to_rgba(dbz));
            }
        }
    }
    encode_png(&rgba)
}

pub fn empty_tile_png() -> &'static [u8] {
    static EMPTY: OnceLock<Vec<u8>> = OnceLock::new();
    EMPTY.get_or_init(|| encode_png(&vec![0u8; (TILE_SIZE * TILE_SIZE * 4) as usize]))
}

fn encode_png(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, TILE_SIZE, TILE_SIZE);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header to memory cannot fail");
        writer.write_image_data(rgba).expect("PNG data to memory cannot fail");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{encode_dbz, Grid};
    use crate::palette::dbz_to_rgba;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    fn decode(png_bytes: &[u8]) -> Vec<u8> {
        let mut reader = png::Decoder::new(png_bytes).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();
        buf
    }

    /// 2°x2° of uniform echo centred near (30, -90).
    fn uniform_grid(dbz: f32) -> Grid {
        Grid {
            nx: 200,
            ny: 200,
            lat0: 31.0,
            lon0: -91.0,
            dlat: 0.01,
            dlon: 0.01,
            values: vec![encode_dbz(dbz); 200 * 200],
        }
    }

    #[test]
    fn world_tile_centre_is_origin() {
        let (lat, lon) = tile_point_to_latlon(0, 0, 0, 0.5, 0.5);
        assert!(close(lat, 0.0) && close(lon, 0.0), "{lat}, {lon}");
    }

    #[test]
    fn top_left_is_mercator_limit() {
        let (lat, lon) = tile_point_to_latlon(1, 0, 0, 0.0, 0.0);
        assert!(close(lat, 85.05113) && close(lon, -180.0), "{lat}, {lon}");
    }

    #[test]
    fn latlon_to_tile_lands_inside_its_tile() {
        for z in [0u8, 4, 7, 12] {
            let (x, y) = latlon_to_tile(z, 29.95, -90.07);
            let (s, w, n, e) = tile_bounds(z, x, y);
            assert!(s <= 29.95 && 29.95 <= n && w <= -90.07 && -90.07 <= e, "z{z}");
        }
    }

    #[test]
    fn valid_tile_ranges() {
        assert!(valid_tile(0, 0, 0));
        assert!(valid_tile(3, 7, 7));
        assert!(!valid_tile(3, 8, 0));
        assert!(!valid_tile(3, 0, 8));
        assert!(!valid_tile(MAX_ZOOM + 1, 0, 0));
    }

    #[test]
    fn renders_palette_colour_where_grid_has_echo() {
        let g = uniform_grid(40.0);
        let (x, y) = latlon_to_tile(8, 30.0, -90.0);
        let rgba = decode(&render_tile(&g, 8, x, y));
        assert!(rgba.chunks(4).any(|p| p == &dbz_to_rgba(40.0)[..]));
    }

    #[test]
    fn tile_outside_grid_is_the_empty_tile() {
        assert_eq!(render_tile(&uniform_grid(40.0), 3, 4, 2), empty_tile_png());
        assert!(decode(empty_tile_png()).iter().all(|&b| b == 0));
    }
}
