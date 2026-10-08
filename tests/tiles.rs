mod common;

use rust_radar::grid::Grid;
use rust_radar::tiles;

#[test]
fn fixture_rain_renders_coloured_pixels() {
    let g = Grid::from_grib2_gz(&common::mrms_fixture()).unwrap();
    let (x, y) = common::rainy_tile(&g, 7);
    let rgba = common::decode_png(&tiles::render_tile(&g, 7, x, y));
    assert_eq!(rgba.len(), 256 * 256 * 4);
    assert!(common::opaque_pixels(&rgba) > 0);
}

#[test]
fn fixture_tile_over_europe_is_transparent() {
    let g = Grid::from_grib2_gz(&common::mrms_fixture()).unwrap();
    assert_eq!(tiles::render_tile(&g, 3, 4, 2), tiles::empty_tile_png());
}
