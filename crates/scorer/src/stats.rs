//! Statistical tests.
//!
//! Each function returns `(p_value, effect_size, passes_floor)`.
//!
//! Effect-size floors:
//!   Chi-squared  → Cramér's V ≥ 0.10
//!   Fisher exact → odds ratio ≥ 1.5 or ≤ 0.67
//!   KS           → D statistic ≥ 0.05
//!   Bootstrap    → standardised effect ≥ 0.20

use statrs::distribution::{ChiSquared, ContinuousCDF};
use crate::{ScoredResult, ScorerError};

// ── chi-squared ───────────────────────────────────────────────────────────────

/// Two-sided chi-squared goodness-of-fit against a uniform null.
///
/// `counts` are the k bin counts; `df` must equal `k - 1`.
pub fn chi_squared_score(
    counts: &[u64],
    sample_size: u64,
    df: u32,
) -> Result<ScoredResult, ScorerError> {
    let k = counts.len();
    if k < 2 {
        return Err(ScorerError::InvalidInput("need at least 2 bins".into()));
    }
    if df as usize != k - 1 {
        return Err(ScorerError::InvalidInput(format!(
            "df={df} but k-1={}",
            k - 1
        )));
    }
    let n = sample_size as f64;
    if n == 0.0 {
        return Err(ScorerError::InvalidInput("sample_size is zero".into()));
    }

    let expected = n / k as f64;
    let chi2: f64 = counts
        .iter()
        .map(|&c| {
            let diff = c as f64 - expected;
            diff * diff / expected
        })
        .sum();

    // p = 1 − CDF(chi2, df)
    let dist = ChiSquared::new(df as f64)
        .map_err(|e| ScorerError::TestFailed(e.to_string()))?;
    let p_value = 1.0 - dist.cdf(chi2);

    // Cramér's V = sqrt(chi2 / (n * (k-1)))
    let v = (chi2 / (n * df as f64)).sqrt();

    Ok(ScoredResult {
        p_value,
        effect_size: v,
        passes_effect_floor: v >= 0.10,
    })
}

// ── Fisher exact ──────────────────────────────────────────────────────────────

/// Two-sided Fisher exact test for a 2×2 table.
///
/// `counts` must be exactly 4 elements: `[a, b, c, d]` where the table is:
///
/// ```text
///      col1  col2
/// row1   a     b
/// row2   c     d
/// ```
pub fn fisher_exact_score(counts: &[u64]) -> Result<ScoredResult, ScorerError> {
    if counts.len() != 4 {
        return Err(ScorerError::InvalidInput(
            "Fisher exact requires exactly 4 counts [a,b,c,d]".into(),
        ));
    }
    let (a, b, c, d) = (counts[0], counts[1], counts[2], counts[3]);
    let n = a + b + c + d;
    if n == 0 {
        return Err(ScorerError::InvalidInput("all counts are zero".into()));
    }

    let p_value = fisher_two_sided(a, b, c, d);

    // Odds ratio (with 0.5 continuity correction to avoid div-by-zero)
    let or = ((a as f64 + 0.5) * (d as f64 + 0.5))
        / ((b as f64 + 0.5) * (c as f64 + 0.5));

    // Floor: OR ≥ 1.5 or OR ≤ 0.67
    let passes = or >= 1.5 || or <= 0.67;

    Ok(ScoredResult {
        p_value,
        effect_size: or,
        passes_effect_floor: passes,
    })
}

/// Two-sided Fisher exact p-value via hypergeometric enumeration.
fn fisher_two_sided(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let r1 = a + b; // row 1 total
    let r2 = c + d; // row 2 total
    let c1 = a + c; // col 1 total
    let n = r1 + r2;

    // P(X = x) = C(r1,x)*C(r2,c1-x) / C(n,c1)
    // where X ~ Hypergeometric(n, c1, r1)
    let x_min = c1.saturating_sub(r2);
    let x_max = r1.min(c1);

    // Compute log-probability for observed table
    let log_p_obs = log_hypergeom_prob(a, r1, c1, n);

    let mut p_value = 0.0f64;
    for x in x_min..=x_max {
        let log_p = log_hypergeom_prob(x, r1, c1, n);
        if log_p <= log_p_obs + 1e-10 {
            p_value += log_p.exp();
        }
    }
    p_value.min(1.0)
}

/// log P(X = x) for Hypergeometric(N, K, n): log[C(K,x)*C(N-K,n-x)/C(N,n)]
fn log_hypergeom_prob(x: u64, n_draws: u64, k_successes: u64, population: u64) -> f64 {
    log_binom(k_successes, x)
        + log_binom(population - k_successes, n_draws - x)
        - log_binom(population, n_draws)
}

/// log C(n, k) using log-gamma
fn log_binom(n: u64, k: u64) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    log_factorial(n) - log_factorial(k) - log_factorial(n - k)
}

/// log n! via Stirling for large n, exact for small n
fn log_factorial(n: u64) -> f64 {
    statrs::function::gamma::ln_gamma(n as f64 + 1.0)
}

// ── Kolmogorov-Smirnov (one-sample vs uniform) ────────────────────────────────

/// One-sample KS test of binned counts against a discrete uniform distribution.
///
/// `counts` are the bin frequencies in order. We compare the empirical CDF of
/// the bin proportions against the theoretical uniform CDF.
pub fn ks_score(counts: &[u64], sample_size: u64) -> Result<ScoredResult, ScorerError> {
    let k = counts.len();
    if k < 2 {
        return Err(ScorerError::InvalidInput("need at least 2 bins".into()));
    }
    let n = sample_size as f64;
    if n == 0.0 {
        return Err(ScorerError::InvalidInput("sample_size is zero".into()));
    }

    // D = max |F_empirical(i/k) - F_uniform(i/k)| over i = 1..k
    let mut d = 0.0f64;
    let mut cumulative: f64 = 0.0;
    for (i, &c) in counts.iter().enumerate() {
        cumulative += c as f64 / n;
        let theoretical = (i + 1) as f64 / k as f64;
        d = d.max((cumulative - theoretical).abs());
    }

    // Asymptotic KS p-value: p = 2 * sum_{j=1}^∞ (-1)^{j+1} exp(-2 j² D² n)
    // For n*D² > 0.5 this converges very quickly (2-3 terms).
    let p_value = ks_p_value_asymptotic(d, n);

    Ok(ScoredResult {
        p_value,
        effect_size: d,
        passes_effect_floor: d >= 0.05,
    })
}

/// Asymptotic KS p-value (upper tail, one-sample).
///
/// P(K > t) = 2 Σ_{j=1}^∞ (-1)^{j+1} exp(-2j²t²)
///
/// The alternating series converges well only for t ≥ ~0.5. Below that,
/// p is effectively 1.0 (the 10% critical value is t ≈ 1.22), so we short-
/// circuit early rather than accumulate floating-point garbage.
fn ks_p_value_asymptotic(d: f64, n: f64) -> f64 {
    let t = d * n.sqrt();
    if t < 0.5 {
        return 1.0;
    }
    let mut p = 0.0f64;
    for j in 1u64..=100 {
        let term = (-2.0 * (j * j) as f64 * t * t).exp();
        let contribution = if j % 2 == 1 { term } else { -term };
        p += 2.0 * contribution;
        if term < 1e-15 {
            break;
        }
    }
    p.clamp(0.0, 1.0)
}

// ── Bootstrap / permutation (stub) ───────────────────────────────────────────

/// Bootstrap permutation test — stub for M2.
///
/// Returns p=1.0 (i.e., not significant) until a real corpus is available.
/// When M3 lands, this will be replaced with a proper permutation test.
pub fn bootstrap_score(
    counts: &[u64],
    sample_size: u64,
    _permutations: u32,
) -> Result<ScoredResult, ScorerError> {
    let k = counts.len();
    if k < 2 || sample_size == 0 {
        return Err(ScorerError::InvalidInput("need at least 2 bins and n>0".into()));
    }

    // Stub: compute the mean absolute deviation from uniform as effect size.
    let n = sample_size as f64;
    let expected = n / k as f64;
    let mad: f64 = counts.iter().map(|&c| (c as f64 - expected).abs()).sum::<f64>()
        / (k as f64 * expected);

    Ok(ScoredResult {
        p_value: 1.0,
        effect_size: mad,
        passes_effect_floor: mad >= 0.20,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Uniform counts → chi-squared should be 0 → p = 1.0.
    #[test]
    fn chi_squared_uniform_is_not_significant() {
        let counts = vec![100u64; 7];
        let r = chi_squared_score(&counts, 700, 6).unwrap();
        assert!(r.p_value > 0.99, "p={}", r.p_value);
        assert!(r.effect_size < 1e-10, "V={}", r.effect_size);
        assert!(!r.passes_effect_floor);
    }

    /// Strongly skewed: all mass in bin 0 → p ≈ 0 and V ≫ 0.10.
    #[test]
    fn chi_squared_skewed_is_significant() {
        let mut counts = vec![0u64; 7];
        counts[0] = 1_000_000;
        let r = chi_squared_score(&counts, 1_000_000, 6).unwrap();
        assert!(r.p_value < 1e-10, "p={}", r.p_value);
        assert!(r.effect_size > 0.10);
        assert!(r.passes_effect_floor);
    }

    /// 2×2 table with no association → p should be large.
    #[test]
    fn fisher_balanced_table() {
        // a=50, b=50, c=50, d=50 — no association
        let r = fisher_exact_score(&[50, 50, 50, 50]).unwrap();
        assert!(r.p_value > 0.90, "p={}", r.p_value);
        // OR ≈ 1 → doesn't pass floor
        assert!(!r.passes_effect_floor);
    }

    /// 2×2 table with very strong association → p ≈ 0.
    #[test]
    fn fisher_strong_association() {
        // a=100, b=0, c=0, d=100
        let r = fisher_exact_score(&[100, 0, 0, 100]).unwrap();
        assert!(r.p_value < 0.001, "p={}", r.p_value);
        assert!(r.passes_effect_floor);
    }

    /// Uniform bins → KS D ≈ 0, p ≈ 1.
    #[test]
    fn ks_uniform_not_significant() {
        let counts = vec![1000u64; 10];
        let r = ks_score(&counts, 10_000).unwrap();
        assert!(r.p_value > 0.90, "p={}", r.p_value);
        assert!(!r.passes_effect_floor);
    }

    /// All mass in first bin → D = (k-1)/k, p ≈ 0.
    #[test]
    fn ks_all_mass_first_bin() {
        let mut counts = vec![0u64; 10];
        counts[0] = 10_000;
        let r = ks_score(&counts, 10_000).unwrap();
        assert!(r.p_value < 0.001, "p={}", r.p_value);
        assert!(r.passes_effect_floor);
    }
}
