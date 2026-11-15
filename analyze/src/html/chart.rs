//! The SVG bar and line charts of the HTML report.

use super::esc;
use crate::dt::Date;
use crate::format::fmt_pace;
use crate::metrics::Week;

/// A line does not join sessions further apart than this.
const LINE_BREAK_DAYS: i64 = 56;

const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 220.0;
const LEFT: f64 = 44.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 16.0;
const BOTTOM: f64 = 24.0;
const MIN_TICK_SPACING: f64 = 64.0;
const GAP: f64 = 2.0;

fn plot_width() -> f64 {
    WIDTH - LEFT - RIGHT
}

fn plot_height() -> f64 {
    HEIGHT - TOP - BOTTOM
}

fn empty_chart(title: &str) -> String {
    format!(
        "<figure class=\"chart\"><figcaption><h3>{}</h3></figcaption><p class=\"empty\">No data \
         in this range.</p></figure>\n",
        esc(title)
    )
}

fn frame(title: &str, caption: &str, legend: &str, svg: &str) -> String {
    format!(
        "<figure class=\"chart\">\n<figcaption><h3>{title}</h3><p>{caption}</p>{legend}\
         </figcaption>\n<div class=\"plot\"><svg viewBox=\"0 0 {WIDTH} {HEIGHT}\" role=\"img\" \
         aria-label=\"{title}\">\n{svg}</svg></div>\n</figure>\n",
        title = esc(title),
        caption = esc(caption),
    )
}

/// `kind` is the legend mark: a filled box for bars, a stroke for lines.
fn legend(items: &[(String, &str, &str)]) -> String {
    let entries: String = items
        .iter()
        .map(|(name, class, kind)| {
            format!(
                "<span class=\"entry\"><span class=\"{kind} {class}\"></span>{}</span>",
                esc(name)
            )
        })
        .collect();
    format!("<div class=\"legend\">{entries}</div>")
}

/// The largest round step that gives at most four intervals over `range`.
fn nice_step(range: f64) -> f64 {
    let raw = range / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|step| *step >= raw * 0.999)
        .unwrap_or(10.0 * magnitude)
}

/// A step that falls on round times, for an axis in seconds.
fn time_step(range: f64) -> f64 {
    [5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
        .into_iter()
        .find(|step| range / step <= 4.0)
        .unwrap_or_else(|| nice_step(range))
}

fn month_name(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES[(month.clamp(1, 12) - 1) as usize]
}

/// A linear date scale over the plot, with labels on month starts.
struct TimeAxis {
    from: i64,
    to: i64,
    left: f64,
    width: f64,
}

impl TimeAxis {
    fn x(&self, days: i64) -> f64 {
        self.left + (days - self.from) as f64 / (self.to - self.from) as f64 * self.width
    }

    fn labels(&self, start: Date) -> String {
        let span = (self.to - self.from) as f64;
        let per_month = self.width * 30.4 / span;
        let every = [1u32, 2, 3, 6, 12]
            .into_iter()
            .find(|months| per_month * f64::from(*months) >= MIN_TICK_SPACING)
            .unwrap_or(12);
        let y = HEIGHT - 6.0;
        let (mut year, mut month) = (start.year, start.month);
        let mut out = String::new();
        let mut first = true;
        loop {
            let date = Date {
                year,
                month,
                day: 1,
            };
            if date.days() > self.to {
                break;
            }
            if date.days() >= self.from && (month - 1) % every == 0 {
                let x = self.x(date.days());
                if x <= WIDTH - RIGHT - 14.0 {
                    let label = if first || month == 1 {
                        format!("{} {year}", month_name(month))
                    } else {
                        month_name(month).to_string()
                    };
                    first = false;
                    out.push_str(&format!(
                        "<text class=\"tick\" x=\"{x:.1}\" y=\"{y}\" text-anchor=\"start\">{label}</text>\n"
                    ));
                }
            }
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        if first {
            out.push_str(&format!(
                "<text class=\"tick\" x=\"{:.1}\" y=\"{y}\" text-anchor=\"start\">{start}</text>\n",
                self.left
            ));
        }
        out
    }
}

/// Gridlines and their labels. `y` maps a value to its height on the plot.
fn grid(ticks: &[f64], y: &dyn Fn(f64) -> f64, show: &dyn Fn(f64) -> String) -> String {
    ticks
        .iter()
        .map(|&tick| {
            let height = y(tick);
            format!(
                "<line class=\"grid\" x1=\"{LEFT}\" x2=\"{:.1}\" y1=\"{height:.1}\" y2=\"{height:.1}\"/>\n\
                 <text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>\n",
                WIDTH - RIGHT,
                LEFT - 6.0,
                height + 4.0,
                esc(&show(tick))
            )
        })
        .collect()
}

/// The name and the weekly value of a line drawn over bars.
type Overlay<'c> = (&'c str, fn(&Week) -> Option<f64>);

pub(super) struct BarChart<'c> {
    pub(super) title: &'c str,
    pub(super) caption: &'c str,
    pub(super) series: &'c [(&'c str, &'c str)],
    /// A line drawn over the bars, in the same unit.
    pub(super) overlay: Option<Overlay<'c>>,
    pub(super) peak: fn(f64) -> String,
}

/// A path for a bar with rounded top corners when `round` is set.
fn bar_path(x: f64, top: f64, width: f64, height: f64, round: bool) -> String {
    let radius = if round {
        4f64.min(width / 2.0).min(height)
    } else {
        0.0
    };
    let bottom = top + height;
    if radius <= 0.0 {
        return format!("M{x:.1} {bottom:.1}V{top:.1}H{:.1}V{bottom:.1}Z", x + width);
    }
    format!(
        "M{x:.1} {bottom:.1}V{:.1}Q{x:.1} {top:.1} {:.1} {top:.1}H{:.1}Q{:.1} {top:.1} {:.1} {:.1}V{bottom:.1}Z",
        top + radius,
        x + radius,
        x + width - radius,
        x + width,
        x + width,
        top + radius
    )
}

/// Stacked bars per week, on one baseline. A week without sessions leaves a gap.
pub(super) fn bar_chart(
    weeks: &[&Week],
    chart: &BarChart,
    values: impl Fn(&Week) -> Vec<f64>,
    tip: impl Fn(&Week) -> String,
) -> String {
    let totals: Vec<f64> = weeks.iter().map(|w| values(w).iter().sum()).collect();
    let overlay_max = chart.overlay.map_or(0.0, |(_, value)| {
        weeks.iter().filter_map(|w| value(w)).fold(0.0, f64::max)
    });
    let peak = totals.iter().copied().fold(0.0, f64::max);
    let (Some(first), Some(last)) = (weeks.first(), weeks.last()) else {
        return empty_chart(chart.title);
    };
    if peak <= 0.0 {
        return empty_chart(chart.title);
    }

    let axis = TimeAxis {
        from: first.monday.days(),
        to: last.monday.days() + 7,
        left: LEFT,
        width: plot_width(),
    };
    let band = plot_width() / weeks.len() as f64;
    let bar_width = (band - GAP).max(band * 0.6).min(24.0);

    let step = nice_step(peak.max(overlay_max));
    let top_value = (peak.max(overlay_max) / step).ceil() * step;
    let scale = plot_height() / top_value;
    let baseline = TOP + plot_height();
    let y = |value: f64| baseline - value * scale;
    let count = (top_value / step).round() as usize;
    let ticks: Vec<f64> = (0..=count).map(|i| i as f64 * step).collect();

    let mut svg = grid(&ticks, &y, &|v| format!("{v:.0}"));
    svg.push_str(&axis.labels(first.monday));
    let mut peak_label: Option<(f64, f64)> = None;
    for (week, total) in weeks.iter().zip(&totals) {
        let slot = axis.x(week.monday.days());
        let x = slot + (band - bar_width) / 2.0;
        let parts = values(week);
        let last_part = parts.iter().rposition(|v| *v > 0.0);
        let mut height_so_far = 0.0;
        for (i, (value, (_, class))) in parts.iter().zip(chart.series).enumerate() {
            if *value <= 0.0 {
                continue;
            }
            let height = value * scale;
            let is_top = last_part == Some(i);
            let drawn = if is_top {
                height
            } else {
                (height - GAP).max(1.0)
            };
            let top = baseline - height_so_far - drawn;
            svg.push_str(&format!(
                "<path class=\"{class}\" d=\"{}\"/>\n",
                bar_path(x, top, bar_width, drawn, is_top)
            ));
            height_so_far += height;
        }
        // peak is the largest of the totals, so this finds that bar exactly.
        #[allow(clippy::float_cmp)]
        if *total == peak && peak_label.is_none() {
            peak_label = Some((x + bar_width / 2.0, y(*total)));
        }
    }
    if let Some((_, value)) = &chart.overlay {
        let mut path = String::new();
        let mut pen_down = false;
        for week in weeks {
            match value(week) {
                Some(v) => {
                    let x = axis.x(week.monday.days()) + band / 2.0;
                    path.push_str(&format!(
                        "{}{x:.1} {:.1}",
                        if pen_down { "L" } else { "M" },
                        y(v)
                    ));
                    pen_down = true;
                }
                None => pen_down = false,
            }
        }
        svg.push_str(&format!("<path class=\"baseline-line\" d=\"{path}\"/>\n"));
    }
    svg.push_str(&format!(
        "<line class=\"axis\" x1=\"{LEFT}\" x2=\"{:.1}\" y1=\"{baseline:.1}\" y2=\"{baseline:.1}\"/>\n",
        WIDTH - RIGHT
    ));
    if let Some((x, top)) = peak_label {
        let x = x.clamp(LEFT + 20.0, WIDTH - RIGHT - 20.0);
        svg.push_str(&format!(
            "<text class=\"peak\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>\n",
            (top - 5.0).max(11.0),
            esc(&(chart.peak)(peak))
        ));
    }
    for week in weeks {
        svg.push_str(&format!(
            "<rect class=\"hit\" x=\"{:.1}\" y=\"{TOP}\" width=\"{band:.1}\" height=\"{:.1}\" data-tip=\"{}\"/>\n",
            axis.x(week.monday.days()),
            plot_height(),
            esc(&tip(week))
        ));
    }

    let mut items: Vec<(String, &str, &str)> = Vec::new();
    if chart.series.len() >= 2 || chart.overlay.is_some() {
        for (name, class) in chart.series {
            items.push((name.to_string(), class, "box"));
        }
    }
    if let Some((name, _)) = &chart.overlay {
        items.push((name.to_string(), "neutral", "stroke"));
    }
    let keys = if items.is_empty() {
        String::new()
    } else {
        legend(&items)
    };
    frame(chart.title, chart.caption, &keys, &svg)
}

pub(super) struct Point {
    pub(super) date: Date,
    pub(super) y: f64,
    pub(super) tip: String,
}

pub(super) struct Series {
    pub(super) name: String,
    pub(super) class: &'static str,
    pub(super) points: Vec<Point>,
}

pub(super) struct YAxis {
    /// Smaller values at the top, as with pace.
    invert: bool,
    /// Values are seconds, so the ticks fall on round times.
    seconds: bool,
    show: fn(f64) -> String,
}

pub(super) const PACE: YAxis = YAxis {
    invert: true,
    seconds: true,
    show: fmt_pace,
};

pub(super) const TWO_DECIMALS: YAxis = YAxis {
    invert: false,
    seconds: false,
    show: |v| format!("{v:.2}"),
};

pub(super) const WHOLE: YAxis = YAxis {
    invert: false,
    seconds: false,
    show: |v| format!("{v:.0}"),
};

/// Lines with a marker on every session, on one date scale and one value axis.
pub(super) fn line_chart(title: &str, caption: &str, series: &[Series], axis_y: &YAxis) -> String {
    let points = || series.iter().flat_map(|s| s.points.iter());
    let (Some(min_date), Some(max_date)) = (
        points().map(|p| p.date).min(),
        points().map(|p| p.date).max(),
    ) else {
        return empty_chart(title);
    };
    let low = points().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let high = points().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let range = (high - low).max(if axis_y.seconds {
        20.0
    } else {
        high.abs() * 0.1 + 0.01
    });
    let step = if axis_y.seconds {
        time_step(range)
    } else {
        nice_step(range)
    };
    let bottom_value = (low / step).floor() * step;
    let top_value = ((high / step).ceil() * step).max(bottom_value + step);
    let count = ((top_value - bottom_value) / step).round() as usize;
    let ticks: Vec<f64> = (0..=count)
        .map(|i| bottom_value + i as f64 * step)
        .collect();
    let fraction = |value: f64| (value - bottom_value) / (top_value - bottom_value);
    let y = |value: f64| {
        let share = if axis_y.invert {
            1.0 - fraction(value)
        } else {
            fraction(value)
        };
        TOP + plot_height() * (1.0 - share)
    };

    // A little room either side, so an end marker does not sit on the plot's edge.
    let pad = ((max_date.days() - min_date.days()) / 40).max(3);
    let axis = TimeAxis {
        from: min_date.days() - pad,
        to: max_date.days() + pad,
        left: LEFT,
        width: plot_width(),
    };

    let mut svg = grid(&ticks, &y, &axis_y.show);
    svg.push_str(&axis.labels(min_date.minus_days(pad)));
    let baseline = TOP + plot_height();
    svg.push_str(&format!(
        "<line class=\"axis\" x1=\"{LEFT}\" x2=\"{:.1}\" y1=\"{baseline:.1}\" y2=\"{baseline:.1}\"/>\n",
        WIDTH - RIGHT
    ));
    for s in series {
        let mut path = String::new();
        let mut previous: Option<Date> = None;
        for point in &s.points {
            let joined = previous.is_some_and(|p| point.date.days() - p.days() <= LINE_BREAK_DAYS);
            path.push_str(&format!(
                "{}{:.1} {:.1}",
                if joined { "L" } else { "M" },
                axis.x(point.date.days()),
                y(point.y)
            ));
            previous = Some(point.date);
        }
        if !path.is_empty() {
            svg.push_str(&format!(
                "<path class=\"line {}\" d=\"{path}\"/>\n",
                s.class
            ));
        }
    }
    for s in series {
        for point in &s.points {
            svg.push_str(&format!(
                "<circle class=\"dot {}\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"4\"/>\n",
                s.class,
                axis.x(point.date.days()),
                y(point.y)
            ));
        }
    }
    for s in series {
        for point in &s.points {
            svg.push_str(&format!(
                "<circle class=\"hit\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"12\" data-tip=\"{}\"/>\n",
                axis.x(point.date.days()),
                y(point.y),
                esc(&point.tip)
            ));
        }
    }
    let visible: Vec<(String, &str, &str)> = series
        .iter()
        .filter(|s| !s.points.is_empty())
        .map(|s| (s.name.clone(), s.class, "stroke"))
        .collect();
    let keys = if visible.len() >= 2 {
        legend(&visible)
    } else {
        // One line of several still needs its name, because the colour alone does not say.
        match (visible.first(), series.len()) {
            (Some(only), n) if n >= 2 => legend(std::slice::from_ref(only)),
            _ => String::new(),
        }
    };
    frame(title, caption, &keys, &svg)
}
