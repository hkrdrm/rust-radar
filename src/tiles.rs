//! Web Mercator tile maths and radar tile rendering.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::grid::{decode_dbz, Grid, NO_COVERAGE};
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

/// Faint diagonal hatch drawn wherever the radar cannot see.
pub const NO_COVERAGE_RGBA: [u8; 4] = [255, 255, 255, 40];
const HATCH_SPACING: usize = 8;

fn no_coverage_pixel(px: usize, py: usize) -> [u8; 4] {
    // Tile sizes are multiples of the spacing, so the hatch lines up across tiles.
    if (px + py).is_multiple_of(HATCH_SPACING) { NO_COVERAGE_RGBA } else { [0, 0, 0, 0] }
}

pub fn render_tile(grid: &Grid, z: u8, x: u32, y: u32) -> Vec<u8> {
    if !tile_overlaps(grid.bounds(), z, x, y) {
        return no_coverage_tile_png().to_vec();
    }
    let size = TILE_SIZE as usize;
    // Pixel edges: latitude depends only on the row, longitude only on the column.
    let edge = |p: usize| p as f64 / size as f64;
    let lons: Vec<f64> = (0..=size).map(|px| tile_point_to_latlon(z, x, y, edge(px), 0.0).1).collect();
    let lats: Vec<f64> = (0..=size).map(|py| tile_point_to_latlon(z, x, y, 0.0, edge(py)).0).collect();
    // When a pixel spans several cells, pool them so small cells don't vanish at low zoom.
    let pool = (lons[1] - lons[0]) > 1.5 * grid.dlon;
    let mut rgba = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let value = if pool {
                grid.strongest_in(lats[py], lats[py + 1], lons[px], lons[px + 1])
            } else {
                grid.cell((lats[py] + lats[py + 1]) / 2.0, (lons[px] + lons[px + 1]) / 2.0)
            };
            let colour = match value {
                None | Some(NO_COVERAGE) => no_coverage_pixel(px, py),
                Some(v) => decode_dbz(v).map_or([0, 0, 0, 0], dbz_to_rgba),
            };
            let i = (py * size + px) * 4;
            rgba[i..i + 4].copy_from_slice(&colour);
        }
    }
    encode_png(&rgba)
}

/// A tile entirely outside radar coverage.
pub fn no_coverage_tile_png() -> &'static [u8] {
    static TILE: OnceLock<Vec<u8>> = OnceLock::new();
    TILE.get_or_init(|| {
        let size = TILE_SIZE as usize;
        let rgba: Vec<u8> = (0..size * size).flat_map(|i| no_coverage_pixel(i % size, i / size)).collect();
        encode_png(&rgba)
    })
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
    use crate::grid::{encode_dbz, Grid, NO_COVERAGE, NO_DATA};
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
    fn tile_outside_grid_is_the_no_coverage_tile() {
        assert_eq!(render_tile(&uniform_grid(40.0), 3, 4, 2), no_coverage_tile_png());
        let rgba = decode(no_coverage_tile_png());
        assert!(rgba.chunks(4).any(|p| p == &NO_COVERAGE_RGBA[..]), "hatched");
        assert!(rgba.chunks(4).any(|p| p[3] == 0), "hatch leaves the map visible");
    }

    #[test]
    fn no_coverage_is_hatched_but_clear_air_is_transparent() {
        let mut g = uniform_grid(0.0);
        g.values = vec![NO_DATA; 200 * 200];
        for row in 0..100 {
            for col in 0..200 {
                g.values[row * 200 + col] = NO_COVERAGE; // northern half beyond coverage
            }
        }
        let z = 9;
        let (x, y) = latlon_to_tile(z, 30.5, -90.0);
        let north = decode(&render_tile(&g, z, x, y));
        assert!(north.chunks(4).any(|p| p == &NO_COVERAGE_RGBA[..]));
        let (x, y) = latlon_to_tile(z + 1, 29.7, -90.0);
        let (s, w, n, e) = tile_bounds(z + 1, x, y);
        assert!(s > 29.05 && n < 29.95 && w > -90.95 && e < -89.05, "test tile must lie inside the clear half");
        let south = decode(&render_tile(&g, z + 1, x, y));
        assert!(south.chunks(4).all(|p| p[3] == 0), "clear air must stay transparent");
    }

    #[test]
    fn low_zoom_keeps_isolated_echo_visible() {
        let mut g = uniform_grid(0.0);
        g.values = vec![NO_DATA; 200 * 200];
        g.values[3 * 200 + 3] = encode_dbz(55.0); // one strong cell near the grid's corner
        let (lat, lon) = g.cell_center(3 * 200 + 3);
        let (x, y) = latlon_to_tile(3, lat, lon);
        let rgba = decode(&render_tile(&g, 3, x, y));
        assert!(rgba.chunks(4).any(|p| p == &dbz_to_rgba(55.0)[..]), "single cell vanished at low zoom");
    }
}
