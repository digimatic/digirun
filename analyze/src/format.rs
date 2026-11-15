//! Number formatting shared by the console report, the CSVs and the HTML page.

use crate::num::round_int;

/// Python's `str(float)`: whole numbers keep the `.0`.
pub fn py_num(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if value.fract() == 0.0 {
        format!("{:.1}", value)
    } else {
        format!("{}", value)
    }
}

/// Pace as `M:SS` per kilometre.
pub fn fmt_pace(seconds: f64) -> String {
    let total = round_int(seconds);
    format!("{}:{:02}", total / 60, total % 60)
}

/// Total time as `H:MM:SS`, or `M:SS` under an hour.
pub fn fmt_duration(seconds: f64) -> String {
    let total = round_int(seconds);
    let (h, rem) = (total / 3600, total % 3600);
    if h != 0 {
        format!("{h}:{:02}:{:02}", rem / 60, rem % 60)
    } else {
        format!("{}:{:02}", rem / 60, rem % 60)
    }
}

/// `fmt_duration`, or `-` for no time.
pub(crate) fn duration_or_dash(seconds: f64) -> String {
    if seconds > 0.0 {
        fmt_duration(seconds)
    } else {
        "-".to_string()
    }
}

/// `+23%`, or `-` when there is nothing to compare with.
pub(crate) fn change(value: f64, baseline: Option<f64>) -> String {
    match baseline {
        Some(baseline) if baseline > 0.0 => {
            format!("{:+}%", round_int((value / baseline - 1.0) * 100.0))
        }
        _ => "-".to_string(),
    }
}
