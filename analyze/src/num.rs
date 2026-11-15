//! Small numeric helpers shared by the analysis and the reports.

/// Python's `round(value, digits)`: the correctly rounded decimal, read back as
/// a float, so the CSVs hold short, stable numbers.
pub fn round(value: f64, digits: usize) -> f64 {
    format!("{:.*}", digits, value)
        .parse::<f64>()
        .unwrap_or(value)
}

/// Python's `round(value)` to an integer (half to even).
pub(crate) fn round_int(value: f64) -> i64 {
    format!("{:.0}", value).parse::<i64>().unwrap_or(0)
}

/// The mean of a non-empty slice.
pub(crate) fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// The mean, or `None` for no values.
pub(crate) fn mean_of(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| mean(values))
}

/// Sample standard deviation (`statistics.stdev`), which needs at least two values.
pub(crate) fn stdev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    let sum: f64 = values.iter().map(|v| (v - m) * (v - m)).sum();
    (sum / (values.len() - 1) as f64).sqrt()
}

/// The median of a non-empty slice, averaging the middle two of an even count.
pub(crate) fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// The upper of the middle two of an even count, so always one of the values;
/// `None` for no values.
pub(crate) fn upper_median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}
