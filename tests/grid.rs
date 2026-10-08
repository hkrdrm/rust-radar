mod common;

use rust_radar::grid::{decode_dbz, Grid};

#[test]
fn decodes_mrms_fixture() {
    let g = Grid::from_grib2_gz(&common::mrms_fixture()).unwrap();
    assert_eq!((g.nx, g.ny), (7000, 3500));
    assert!((g.lat0 - 54.995).abs() < 1e-6, "lat0 = {}", g.lat0);
    assert!((g.lon0 + 129.995).abs() < 1e-6, "lon0 = {}", g.lon0);
    assert!((g.dlat - 0.01).abs() < 1e-6, "dlat = {}", g.dlat);
    assert!((g.dlon - 0.01).abs() < 1e-6, "dlon = {}", g.dlon);
    let (s, w, n, e) = g.bounds();
    assert!((s - 20.0).abs() < 1e-3 && (n - 55.0).abs() < 1e-3, "lat bounds {s}..{n}");
    assert!((w + 130.0).abs() < 1e-3 && (e + 60.0).abs() < 1e-3, "lon bounds {w}..{e}");
    let max = g.values.iter().filter_map(|&v| decode_dbz(v)).fold(0.0f32, f32::max);
    assert!(max > 20.0 && max <= 90.0, "max dBZ {max}");
    common::rainy_cell(&g);
}

#[test]
fn rejects_garbage() {
    assert!(Grid::from_grib2_gz(b"definitely not gzip").is_err());
}

#[test]
fn rejects_truncated_file() {
    let bytes = common::mrms_fixture();
    assert!(Grid::from_grib2_gz(&bytes[..bytes.len() / 2]).is_err());
}
