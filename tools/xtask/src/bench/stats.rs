//! Order statistics over benchmark samples.

/// The median of `samples`; the mean of the two middle values for an even count. `None` when
/// empty.
pub fn median(samples: &[f64]) -> Option<f64> {
    let sorted = sorted(samples);
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2]),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0),
    }
}

/// The nearest-rank percentile (`p` in 1..=100): the smallest sample such that at least `p`
/// percent of the samples are at or below it. `None` when empty.
pub fn percentile(samples: &[f64], p: u32) -> Option<f64> {
    let sorted = sorted(samples);
    if sorted.is_empty() {
        return None;
    }
    let p = usize::try_from(p.clamp(1, 100)).unwrap_or(100);
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1])
}

/// The largest sample.
pub fn max(samples: &[f64]) -> Option<f64> {
    sorted(samples).last().copied()
}

fn sorted(samples: &[f64]) -> Vec<f64> {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_return_middle_value_when_count_is_odd() {
        // given
        let samples = [9.0, 1.0, 5.0];

        // when
        let median = median(&samples);

        // then
        assert_eq!(median, Some(5.0));
    }

    #[test]
    fn should_average_middle_pair_when_count_is_even() {
        // given
        let samples = [4.0, 1.0, 2.0, 10.0];

        // when
        let median = median(&samples);

        // then
        assert_eq!(median, Some(3.0));
    }

    #[test]
    fn should_return_none_when_samples_are_empty() {
        // given
        let samples: [f64; 0] = [];

        // when
        let results = (median(&samples), percentile(&samples, 95), max(&samples));

        // then
        assert_eq!(results, (None, None, None));
    }

    #[test]
    fn should_use_nearest_rank_when_percentile_is_taken() {
        // given
        let samples: Vec<f64> = (1..=50).map(f64::from).collect();

        // when
        let p95 = percentile(&samples, 95);
        let p100 = percentile(&samples, 100);
        let p1 = percentile(&samples, 1);

        // then
        assert_eq!(p95, Some(48.0));
        assert_eq!(p100, Some(50.0));
        assert_eq!(p1, Some(1.0));
    }

    #[test]
    fn should_find_largest_when_samples_are_unordered() {
        // given
        let samples = [3.0, -1.0, 7.5];

        // when
        let largest = max(&samples);

        // then
        assert_eq!(largest, Some(7.5));
    }
}
