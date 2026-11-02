//! Shared integer math: one implementation on every target (spec: The fix type).
//!
//! Tables were generated once from `round(sin(d°) × 65536)` and
//! `round(atan(i / 256) in degrees × 65536)`; they are data, not computed at run time.

/// sin(d°) for d = 0..90, in 16.16.
pub const SIN: [i32; 91] = [
    0, 1144, 2287, 3430, 4572, 5712, 6850, 7987, 9121, 10252, 11380, 12505, 13626, 14742, 15855, 16962, 18064, 19161, 20252, 21336, 22415,
    23486, 24550, 25607, 26656, 27697, 28729, 29753, 30767, 31772, 32768, 33754, 34729, 35693, 36647, 37590, 38521, 39441, 40348, 41243,
    42126, 42995, 43852, 44695, 45525, 46341, 47143, 47930, 48703, 49461, 50203, 50931, 51643, 52339, 53020, 53684, 54332, 54963, 55578,
    56175, 56756, 57319, 57865, 58393, 58903, 59396, 59870, 60326, 60764, 61183, 61584, 61966, 62328, 62672, 62997, 63303, 63589, 63856,
    64104, 64332, 64540, 64729, 64898, 65048, 65177, 65287, 65376, 65446, 65496, 65526, 65536,
];

/// atan(i / 256) in degrees for i = 0..256, in 16.16.
pub const ATAN: [i32; 257] = [
    0, 14668, 29335, 44001, 58666, 73329, 87990, 102648, 117304, 131955, 146603, 161246, 175884, 190517, 205144, 219765, 234379, 248986,
    263585, 278177, 292760, 307334, 321899, 336454, 350999, 365534, 380058, 394570, 409070, 423558, 438034, 452496, 466945, 481380, 495801,
    510207, 524598, 538973, 553333, 567676, 582003, 596312, 610605, 624879, 639135, 653372, 667591, 681790, 695970, 710129, 724268, 738387,
    752484, 766560, 780613, 794645, 808654, 822641, 836604, 850544, 864460, 878352, 892219, 906062, 919879, 933671, 947438, 961178, 974893,
    988580, 1002241, 1015875, 1029481, 1043060, 1056611, 1070133, 1083627, 1097092, 1110529, 1123936, 1137313, 1150661, 1163979, 1177267,
    1190524, 1203751, 1216947, 1230111, 1243245, 1256347, 1269417, 1282455, 1295461, 1308435, 1321376, 1334285, 1347161, 1360004, 1372813,
    1385590, 1398332, 1411041, 1423717, 1436358, 1448965, 1461538, 1474076, 1486580, 1499049, 1511483, 1523882, 1536246, 1548575, 1560868,
    1573127, 1585349, 1597536, 1609687, 1621803, 1633882, 1645926, 1657933, 1669904, 1681839, 1693738, 1705600, 1717426, 1729215, 1740967,
    1752683, 1764362, 1776004, 1787610, 1799179, 1810710, 1822205, 1833663, 1845084, 1856467, 1867814, 1879123, 1890396, 1901631, 1912829,
    1923990, 1935113, 1946200, 1957249, 1968261, 1979236, 1990173, 2001074, 2011937, 2022763, 2033552, 2044303, 2055018, 2065695, 2076336,
    2086939, 2097505, 2108034, 2118526, 2128981, 2139399, 2149780, 2160125, 2170432, 2180703, 2190937, 2201134, 2211295, 2221419, 2231507,
    2241558, 2251572, 2261551, 2271492, 2281398, 2291267, 2301101, 2310898, 2320659, 2330384, 2340074, 2349727, 2359345, 2368927, 2378474,
    2387985, 2397460, 2406901, 2416306, 2425675, 2435010, 2444310, 2453574, 2462804, 2471999, 2481159, 2490285, 2499376, 2508433, 2517455,
    2526443, 2535397, 2544317, 2553203, 2562055, 2570873, 2579658, 2588409, 2597126, 2605811, 2614461, 2623079, 2631664, 2640215, 2648734,
    2657220, 2665673, 2674093, 2682482, 2690837, 2699161, 2707452, 2715711, 2723939, 2732134, 2740298, 2748430, 2756531, 2764600, 2772638,
    2780644, 2788620, 2796564, 2804478, 2812361, 2820213, 2828035, 2835826, 2843587, 2851318, 2859019, 2866690, 2874330, 2881941, 2889523,
    2897075, 2904597, 2912090, 2919554, 2926989, 2934395, 2941772, 2949120,
];

pub const TURN: i64 = 360 * 65536;
const HALF: i32 = 180 * 65536;
const QUARTER: i32 = 90 * 65536;

/// (a * b) >> 16 with a 64-bit intermediate.
#[inline]
pub fn fmul(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 16) as i32
}

/// (a << 16) / b truncated toward zero; `None` for b = 0.
#[inline]
pub fn fdiv(a: i32, b: i32) -> Option<i32> {
    if b == 0 {
        None
    } else {
        Some((((a as i64) << 16) / b as i64) as i32)
    }
}

/// Sine of an angle in degrees (16.16), result in 16.16. Exact at multiples of
/// 90°, odd, and symmetric about 90°.
pub fn sin(a: i32) -> i32 {
    let mut a = (a as i64).rem_euclid(TURN) as i32;
    let mut sign = 1;
    if a >= HALF {
        a -= HALF;
        sign = -1;
    }
    if a > QUARTER {
        a = HALF - a;
    }
    let d = (a >> 16) as usize;
    let f = (a & 0xFFFF) as i64;
    let v = if d >= 90 {
        SIN[90]
    } else {
        SIN[d] + (((SIN[d + 1] - SIN[d]) as i64 * f) >> 16) as i32
    };
    sign * v
}

pub fn cos(a: i32) -> i32 {
    // reduce first so the +90° can't overflow
    let a = (a as i64).rem_euclid(TURN) as i32;
    sin(a + QUARTER)
}

/// atan(r) in degrees for r in 0..=1 (16.16).
fn atan_unit(r: u32) -> i32 {
    let i = (r >> 8) as usize;
    if i >= 256 {
        return ATAN[256];
    }
    let f = (r & 0xFF) as i32;
    ATAN[i] + (((ATAN[i + 1] - ATAN[i]) * f) >> 8)
}

/// Angle of (x, y) in degrees, 0 to under 360 (16.16).
pub fn atan2(y: i32, x: i32) -> i32 {
    if x == 0 && y == 0 {
        return 0;
    }
    let ax = (x as i64).unsigned_abs();
    let ay = (y as i64).unsigned_abs();
    let t = if ay <= ax {
        atan_unit(((ay << 16) / ax) as u32)
    } else {
        QUARTER - atan_unit(((ax << 16) / ay) as u32)
    };
    let a = match (x >= 0, y >= 0) {
        (true, true) => t,
        (false, true) => HALF - t,
        (false, false) => HALF + t,
        (true, false) => 2 * HALF - t,
    };
    if a >= 2 * HALF {
        0
    } else {
        a
    }
}

/// Square root of a 16.16 value, rounded down; 0 for x <= 0.
pub fn sqrt(x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }
    let n = (x as u64) << 16;
    // integer square root by Newton's method, exact floor
    let mut r = 1u64 << ((64 - n.leading_zeros()).div_ceil(2));
    loop {
        let s = (r + n / r) / 2;
        if s >= r {
            break;
        }
        r = s;
    }
    while r * r > n {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= n {
        r += 1;
    }
    r as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    const ONE: i32 = 65536;
    #[test]
    fn sin_exact_points_and_symmetry() {
        assert_eq!(sin(0), 0);
        assert_eq!(sin(90 * ONE), ONE);
        assert_eq!(sin(180 * ONE), 0);
        assert_eq!(sin(270 * ONE), -ONE);
        assert_eq!(cos(0), ONE);
        assert_eq!(cos(180 * ONE), -ONE);
        for a in (-1000 * ONE..1000 * ONE).step_by(12345) {
            assert_eq!(sin(-a), -sin(a), "odd at {}", a);
            assert_eq!(sin(180 * ONE - a), sin(a), "symmetric at {}", a);
        }
        assert_eq!(sin(i32::MIN), sin((i32::MIN as i64).rem_euclid(TURN) as i32));
        assert_eq!(cos(i32::MAX), sin(((i32::MAX as i64).rem_euclid(TURN) + 90 * 65536) as i32));
    }
    #[test]
    fn atan2_quadrants() {
        assert_eq!(atan2(0, ONE), 0);
        assert_eq!(atan2(ONE, 0), 90 * ONE);
        assert_eq!(atan2(0, -ONE), 180 * ONE);
        assert_eq!(atan2(-ONE, 0), 270 * ONE);
        assert_eq!(atan2(ONE, ONE), 45 * ONE);
        let a = atan2(-ONE, ONE);
        assert_eq!(a, 315 * ONE);
        assert!((atan2(ONE, 2 * ONE) - 1740967).abs() < 8); // 26.565°
    }
    #[test]
    fn sqrt_floor() {
        assert_eq!(sqrt(4 * ONE), 2 * ONE);
        assert_eq!(sqrt(2 * ONE), 92681);
        assert_eq!(sqrt(-5), 0);
        assert_eq!(sqrt(i32::MAX), 11863283);
    }
}
