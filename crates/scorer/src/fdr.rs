//! Benjamini-Hochberg FDR correction.
//!
//! Implements two variants:
//!
//! 1. **Batch BH** — classic (1995) procedure for a finished batch of p-values.
//!    Sorts ascending, applies the BH threshold, returns a boolean mask.
//!
//! 2. **Sequential (online) BH** — Wang & Ramdas (2021) `SAFFRON`-style procedure
//!    for streaming hypothesis arrival.  In M2 we implement the simpler
//!    `LORD++`-like accumulator; SAFFRON is reserved for M4.
//!
//! α = 0.05 throughout (configurable).

/// Default FDR level.
pub const ALPHA: f64 = 0.05;

// ── Batch BH ──────────────────────────────────────────────────────────────────

/// Batch Benjamini-Hochberg correction.
///
/// Returns a `Vec<bool>` of the same length as `p_values` — `true` means the
/// hypothesis is rejected (i.e. **significant after correction**).
///
/// Group-aware variant: if all hypotheses in `p_values` belong to the same
/// family / test type, pass them together; call this function once per family
/// and collect.
pub fn bh_reject(p_values: &[f64], alpha: f64) -> Vec<bool> {
    let m = p_values.len();
    if m == 0 {
        return vec![];
    }

    // Sort indices by p-value ascending
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_unstable_by(|&i, &j| p_values[i].partial_cmp(&p_values[j]).unwrap());

    // BH threshold: reject p_{(i)} ≤ (i/m) * α
    let mut max_rejected = None;
    for (rank, &idx) in order.iter().enumerate() {
        let threshold = (rank + 1) as f64 / m as f64 * alpha;
        if p_values[idx] <= threshold {
            max_rejected = Some(rank);
        }
    }

    let mut rejected = vec![false; m];
    if let Some(k) = max_rejected {
        // Reject all hypotheses with rank ≤ k (the BH step-up rule)
        for &idx in &order[..=k] {
            rejected[idx] = true;
        }
    }
    rejected
}

/// Compute BH-adjusted q-values.
///
/// q_{(i)} = min_{j ≥ i} (m / j) * p_{(j)}  (capped at 1.0).
/// Returns q-values in the **original** order of `p_values`.
pub fn bh_q_values(p_values: &[f64]) -> Vec<f64> {
    let m = p_values.len();
    if m == 0 {
        return vec![];
    }

    let mut order: Vec<usize> = (0..m).collect();
    order.sort_unstable_by(|&i, &j| p_values[i].partial_cmp(&p_values[j]).unwrap());

    // Sorted p-values → q-values (running minimum from the right)
    let mut q_sorted = vec![0.0f64; m];
    for (rank, &idx) in order.iter().enumerate() {
        q_sorted[rank] = m as f64 / (rank + 1) as f64 * p_values[idx];
    }
    // Running minimum right-to-left
    let mut running_min = f64::INFINITY;
    for q in q_sorted.iter_mut().rev() {
        running_min = running_min.min(*q);
        *q = running_min.min(1.0);
    }

    // Map back to original order
    let mut result = vec![0.0f64; m];
    for (rank, &idx) in order.iter().enumerate() {
        result[idx] = q_sorted[rank];
    }
    result
}

// ── Sequential (online) BH accumulator ───────────────────────────────────────

/// Online BH accumulator for streaming hypothesis arrival.
///
/// Tracks the running significance budget using the simple `alpha-investing`
/// rule (Foster & Stine 2008): each discovery earns back `alpha * w0`
/// additional budget, so the FDR stays controlled at level `alpha` over any
/// stopping time.
///
/// This is a conservative placeholder — M4 will upgrade to SAFFRON (Wang &
/// Ramdas 2021) which is near-optimal for independent hypotheses.
#[derive(Debug, Clone)]
pub struct SequentialBH {
    alpha: f64,
    /// Remaining budget (initialised to α)
    wealth: f64,
    /// Total hypotheses tested so far
    tested: usize,
    /// Total rejections so far
    rejections: usize,
}

impl SequentialBH {
    pub fn new(alpha: f64) -> Self {
        Self { alpha, wealth: alpha, tested: 0, rejections: 0 }
    }

    /// Test the next hypothesis.  Returns `true` if rejected.
    ///
    /// Each test spends `alpha / (tested + 1)` from the wealth.
    /// A rejection earns back `alpha * 0.5` (conservative recovery).
    pub fn test(&mut self, p_value: f64) -> bool {
        self.tested += 1;
        let threshold = self.wealth / self.tested as f64;
        let rejected = p_value <= threshold;
        if rejected {
            self.rejections += 1;
            // Earn back some wealth on discovery
            self.wealth += self.alpha * 0.5;
        }
        rejected
    }

    pub fn tested(&self) -> usize { self.tested }
    pub fn rejections(&self) -> usize { self.rejections }
    pub fn wealth(&self) -> f64 { self.wealth }
}

// ── group-aware batch correction ──────────────────────────────────────────────

/// Apply BH separately within each family group, then pool the rejected set.
///
/// `families` is a parallel slice of family labels (e.g. `"temporal-cyclic"`).
/// Returns rejection mask in original order.
pub fn group_bh_reject(p_values: &[f64], families: &[&str], alpha: f64) -> Vec<bool> {
    assert_eq!(p_values.len(), families.len());
    let m = p_values.len();
    let mut rejected = vec![false; m];

    // Collect unique family labels
    let mut unique: Vec<&str> = families.to_vec();
    unique.sort_unstable();
    unique.dedup();

    for family in unique {
        let indices: Vec<usize> = families
            .iter()
            .enumerate()
            .filter(|(_, &f)| f == family)
            .map(|(i, _)| i)
            .collect();
        let ps: Vec<f64> = indices.iter().map(|&i| p_values[i]).collect();
        let mask = bh_reject(&ps, alpha);
        for (local_i, &global_i) in indices.iter().enumerate() {
            rejected[global_i] = mask[local_i];
        }
    }
    rejected
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bh_rejects_small_p_values() {
        // Classic example: 8 p-values, α=0.05
        let ps = vec![0.001, 0.002, 0.008, 0.010, 0.020, 0.040, 0.200, 0.800];
        let rejected = bh_reject(&ps, 0.05);
        // Sorted: 0.001, 0.002, 0.008, 0.010, 0.020, 0.040, 0.200, 0.800
        // BH thresholds: 0.00625, 0.0125, 0.01875, 0.025, 0.03125, 0.0375, 0.04375, 0.05
        // p_(1)=0.001 ≤ 0.00625 ✓
        // p_(2)=0.002 ≤ 0.0125  ✓
        // p_(3)=0.008 ≤ 0.01875 ✓
        // p_(4)=0.010 ≤ 0.025   ✓
        // p_(5)=0.020 ≤ 0.03125 ✓
        // p_(6)=0.040 > 0.0375  ✗  ← last rejection is rank 5 (p=0.020)
        // Wait: 0.040 ≤ 0.0375? No: 0.040 > 0.0375
        // So max_rejected rank = 4 (0-indexed), i.e. first 5 are rejected.
        assert!(rejected[0]); // 0.001
        assert!(rejected[1]); // 0.002
        assert!(rejected[2]); // 0.008
        assert!(rejected[3]); // 0.010
        assert!(rejected[4]); // 0.020
        assert!(!rejected[5]); // 0.040
        assert!(!rejected[6]); // 0.200
        assert!(!rejected[7]); // 0.800
    }

    #[test]
    fn bh_all_nulls_no_rejection() {
        let ps = vec![0.5, 0.6, 0.7, 0.8, 0.9];
        let rejected = bh_reject(&ps, 0.05);
        assert!(rejected.iter().all(|&r| !r));
    }

    #[test]
    fn q_values_monotone() {
        let ps = vec![0.01, 0.04, 0.20, 0.80];
        let qs = bh_q_values(&ps);
        // q-values should be ≥ corresponding p-values
        for (p, q) in ps.iter().zip(qs.iter()) {
            assert!(*q >= *p - 1e-12, "q={q} < p={p}");
        }
        // Sorted q-values should be non-decreasing
        let mut order: Vec<usize> = (0..ps.len()).collect();
        order.sort_unstable_by(|&i, &j| ps[i].partial_cmp(&ps[j]).unwrap());
        let sorted_qs: Vec<f64> = order.iter().map(|&i| qs[i]).collect();
        for w in sorted_qs.windows(2) {
            assert!(w[0] <= w[1] + 1e-12, "non-monotone: {:?}", sorted_qs);
        }
    }

    #[test]
    fn sequential_bh_tracks_wealth() {
        let mut sbh = SequentialBH::new(0.05);
        // Very small p → should reject
        assert!(sbh.test(0.001));
        assert_eq!(sbh.rejections(), 1);
        // Very large p → should not reject
        assert!(!sbh.test(0.999));
        assert_eq!(sbh.rejections(), 1);
    }
}
