//! Pure conversions between UI slider fractions and device units.

/// Inclusive `(min, max)` range.
pub type Range = (u16, u16);

fn lerp(range: Range, fraction: f64) -> f64 {
    let (lo, hi) = (f64::from(range.0), f64::from(range.1));
    lo + fraction.clamp(0.0, 1.0) * (hi - lo)
}

fn unlerp(range: Range, value: u16) -> f64 {
    let (lo, hi) = (f64::from(range.0), f64::from(range.1));
    if hi <= lo {
        return 0.0;
    }
    ((f64::from(value) - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Brightness fraction (0 = dimmest, 1 = brightest) to lumen.
pub fn lumen_from_fraction(range: Range, fraction: f64) -> u16 {
    lerp(range, fraction).round() as u16
}

pub fn fraction_from_lumen(range: Range, lumen: u16) -> f64 {
    unlerp(range, lumen)
}

/// Warmth fraction (0 = coolest, 1 = warmest) to Kelvin, snapped to the 100 K
/// steps the device accepts.
pub fn kelvin_from_warmth(range: Range, warmth: f64) -> u16 {
    let kelvin = lerp(range, 1.0 - warmth.clamp(0.0, 1.0));
    let snapped = ((kelvin / 100.0).round() * 100.0) as u16;
    snapped.clamp(range.0, range.1)
}

pub fn warmth_from_kelvin(range: Range, kelvin: u16) -> f64 {
    1.0 - unlerp(range, kelvin)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUMEN: Range = (20, 250);
    const KELVIN: Range = (2700, 6500);

    #[test]
    fn brightness_endpoints_and_roundtrip() {
        assert_eq!(lumen_from_fraction(LUMEN, 0.0), 20);
        assert_eq!(lumen_from_fraction(LUMEN, 1.0), 250);
        assert_eq!(lumen_from_fraction(LUMEN, 7.0), 250, "clamps high");
        assert_eq!(lumen_from_fraction(LUMEN, -1.0), 20, "clamps low");
        for l in 20..=250 {
            assert_eq!(lumen_from_fraction(LUMEN, fraction_from_lumen(LUMEN, l)), l);
        }
    }

    #[test]
    fn warmth_is_inverse_of_kelvin() {
        assert_eq!(kelvin_from_warmth(KELVIN, 0.0), 6500);
        assert_eq!(kelvin_from_warmth(KELVIN, 1.0), 2700);
        assert_eq!(warmth_from_kelvin(KELVIN, 2700), 1.0);
        assert_eq!(warmth_from_kelvin(KELVIN, 6500), 0.0);
    }

    #[test]
    fn kelvin_is_always_a_valid_device_value() {
        for i in 0..=1000 {
            let k = kelvin_from_warmth(KELVIN, f64::from(i) / 1000.0);
            assert_eq!(k % 100, 0, "{k} not a multiple of 100");
            assert!((2700..=6500).contains(&k));
        }
    }

    #[test]
    fn degenerate_range_does_not_divide_by_zero() {
        assert_eq!(fraction_from_lumen((50, 50), 50), 0.0);
    }
}
