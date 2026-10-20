//! Integer fixed-point math for building sampling tables.
//!
//! Poisson arrivals need `e^-λ` and Zipf destinations need `k^-s` for a
//! fractional `s`. `f64::exp` and `f64::powf` go through the platform's libm,
//! whose last-bit results differ between Linux, Windows, macOS and `wasm32`,
//! which would break the cross-platform state hash (ADR 0002 §1). These
//! routines use only integer arithmetic, so every platform builds bit-identical
//! tables. Precision (about 1e-9) matters for calibration, not determinism.
//!
//! Values are unsigned Q32.32: `ONE = 1 << 32` represents 1.0.

/// 1.0 in Q32.32.
pub const ONE: u64 = 1 << 32;

/// ln 2 in Q32.32 (0.693147180559945... x 2^32, rounded).
const LN2: u64 = 2_977_044_472;

/// `e^-x` for `x >= 0` in Q32.32.
///
/// Range reduction: `x = n ln2 + r` with `0 <= r < ln2`, so
/// `e^-x = 2^-n e^-r`, and `e^-r` comes from its Taylor series, which
/// converges fast because `r < 0.7`.
#[must_use]
pub fn exp_neg(x: u64) -> u64 {
    let n = x / LN2;
    if n >= 40 {
        // Below 2^-40: indistinguishable from 0 at Q32 precision.
        return 0;
    }
    let r = i128::from(x % LN2);
    let one = i128::from(ONE);
    // Taylor series of e^-r: sum of (-r)^k / k!.
    let mut term = one;
    let mut sum = one;
    for k in 1..=24i128 {
        term = -term * r / one / k;
        if term == 0 {
            break;
        }
        sum += term;
    }
    // 0 < e^-r <= 1, so sum fits in u64.
    let e_r = u64::try_from(sum.clamp(0, one)).unwrap_or(ONE);
    e_r >> n
}

/// `ln(x)` for an integer `x >= 1`, in Q32.32.
///
/// Write `x = 2^k m` with `1 <= m < 2`; then `ln x = k ln2 + ln m` and
/// `ln m = 2 atanh(z)` with `z = (m - 1) / (m + 1) < 1/3`, whose odd power
/// series converges quickly.
#[must_use]
pub fn ln(x: u64) -> u64 {
    debug_assert!(x >= 1, "ln(0) is undefined");
    let k = 63 - u64::from(x.max(1).leading_zeros());
    let one = i128::from(ONE);
    // m in Q32, in [ONE, 2 ONE).
    let m = (i128::from(x) << 32) >> k;
    let z = (m - one) * one / (m + one);
    let z2 = z * z / one;
    let mut power = z;
    let mut sum = 0i128;
    let mut d = 1i128;
    while power != 0 {
        sum += power / d;
        power = power * z2 / one;
        d += 2;
    }
    k * LN2 + u64::try_from(2 * sum).unwrap_or(0)
}

/// `base^(-s_milli / 1000)` for an integer `base >= 1`, in Q32.32.
#[must_use]
pub fn pow_neg_milli(base: u64, s_milli: u32) -> u64 {
    let exponent = u128::from(ln(base)) * u128::from(s_milli) / 1_000;
    exp_neg(u64::try_from(exponent).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute error tolerance: 1e-8 in Q32 is about 43 units.
    const TOL: u64 = 64;

    fn close(a: u64, b: u64) -> bool {
        a.abs_diff(b) <= TOL
    }

    #[test]
    fn exp_neg_matches_reference_values() {
        assert_eq!(exp_neg(0), ONE);
        // e^-1 = 0.36787944117144233
        assert!(close(exp_neg(ONE), 1_580_030_169), "{}", exp_neg(ONE));
        // e^-0.5 = 0.6065306597126334
        assert!(
            close(exp_neg(ONE / 2), 2_605_029_347),
            "{}",
            exp_neg(ONE / 2)
        );
        // e^-5 = 0.006737946999085467
        assert!(close(exp_neg(5 * ONE), 28_939_262), "{}", exp_neg(5 * ONE));
        assert_eq!(exp_neg(100 * ONE), 0);
    }

    #[test]
    fn ln_matches_reference_values() {
        assert_eq!(ln(1), 0);
        assert!(close(ln(2), LN2), "{}", ln(2));
        // ln 10 = 2.302585092994046
        assert!(close(ln(10), 9_889_527_671), "{}", ln(10));
        // ln 300 = 5.703782474656201
        assert!(close(ln(300), 24_497_559_192), "{}", ln(300));
    }

    #[test]
    fn pow_neg_matches_reference_values() {
        assert_eq!(pow_neg_milli(7, 0), ONE);
        // 2^-1 = 0.5
        assert!(close(pow_neg_milli(2, 1_000), ONE / 2));
        // 10^-0.8 = 0.15848931924611134
        assert!(
            close(pow_neg_milli(10, 800), 680_706_443),
            "{}",
            pow_neg_milli(10, 800)
        );
    }
}
