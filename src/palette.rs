//! NWS-style reflectivity colour scale.

/// (lower bound in dBZ, colour) in ascending order.
const STOPS: [(f32, [u8; 3]); 15] = [
    (5.0, [4, 233, 231]),
    (10.0, [1, 159, 244]),
    (15.0, [3, 0, 244]),
    (20.0, [2, 253, 2]),
    (25.0, [1, 197, 1]),
    (30.0, [0, 142, 0]),
    (35.0, [253, 248, 2]),
    (40.0, [229, 188, 0]),
    (45.0, [253, 149, 0]),
    (50.0, [253, 0, 0]),
    (55.0, [212, 0, 0]),
    (60.0, [188, 0, 0]),
    (65.0, [248, 0, 253]),
    (70.0, [152, 84, 198]),
    (75.0, [253, 253, 253]),
];

pub fn dbz_to_rgba(dbz: f32) -> [u8; 4] {
    if !dbz.is_finite() || dbz < STOPS[0].0 {
        return [0, 0, 0, 0];
    }
    let mut colour = STOPS[0].1;
    for (threshold, rgb) in STOPS {
        if dbz < threshold {
            break;
        }
        colour = rgb;
    }
    [colour[0], colour[1], colour[2], 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

    #[test]
    fn sentinels_and_weak_echo_are_transparent() {
        assert_eq!(dbz_to_rgba(-999.0), TRANSPARENT);
        assert_eq!(dbz_to_rgba(-99.0), TRANSPARENT);
        assert_eq!(dbz_to_rgba(4.9), TRANSPARENT);
        assert_eq!(dbz_to_rgba(f32::NAN), TRANSPARENT);
    }

    #[test]
    fn steps_follow_nws_scale() {
        assert_eq!(dbz_to_rgba(5.0), [4, 233, 231, 255]);
        assert_eq!(dbz_to_rgba(22.0), [2, 253, 2, 255]);
        assert_eq!(dbz_to_rgba(50.0), [253, 0, 0, 255]);
        assert_eq!(dbz_to_rgba(75.0), [253, 253, 253, 255]);
        assert_eq!(dbz_to_rgba(90.0), [253, 253, 253, 255]);
    }
}
