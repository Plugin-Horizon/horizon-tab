//! `/tab cpu`: TAB's `CpuManager` and `CpuCommand`. Tracking starts with the first `/tab cpu`;
//! every ten seconds the time each feature (and each placeholder) took is turned into a share of
//! the window, and the command prints the last such report. The timers are the mod's own
//! (`Instant`), taken around the engine's stages and around each placeholder evaluation.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// The length of one report window.
pub const WINDOW_MS: i64 = 10_000;

/// A finished window as percentages of its length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// feature -> category -> percent.
    pub features: BTreeMap<String, BTreeMap<String, f64>>,
    /// placeholder -> percent, biggest first.
    pub placeholders: Vec<(String, f64)>,
}

impl Report {
    pub fn feature_total(&self) -> f64 {
        self.features.values().flat_map(|c| c.values()).sum()
    }
    pub fn placeholder_total(&self) -> f64 {
        self.placeholders.iter().map(|(_, p)| p).sum()
    }
}

#[derive(Default)]
struct Inner {
    tracking: bool,
    window_start_ms: i64,
    features: HashMap<(String, String), u64>,
    last: Option<Report>,
}

/// The timers. Interior mutability, so the engine can time stages while it reads its state.
#[derive(Default)]
pub struct Cpu {
    inner: RefCell<Inner>,
    lap_at: Cell<Option<Instant>>,
}

impl Cpu {
    pub fn tracking(&self) -> bool {
        self.inner.borrow().tracking
    }

    /// Starts tracking; false when it already runs.
    pub fn enable(&self, now_ms: i64) -> bool {
        let mut i = self.inner.borrow_mut();
        if i.tracking {
            return false;
        }
        i.tracking = true;
        i.window_start_ms = now_ms;
        true
    }

    /// Adds time to a feature's category.
    pub fn add(&self, feature: &str, category: &str, took: Duration) {
        let mut i = self.inner.borrow_mut();
        if !i.tracking {
            return;
        }
        *i.features.entry((feature.to_owned(), category.to_owned())).or_insert(0) += took.as_nanos() as u64;
    }

    /// Marks the start of a stage; `lap` adds the time since it (or the last lap).
    pub fn start_lap(&self) {
        if self.tracking() {
            self.lap_at.set(Some(Instant::now()));
        }
    }

    pub fn lap(&self, feature: &str, category: &str) {
        if let Some(at) = self.lap_at.get() {
            let now = Instant::now();
            self.add(feature, category, now.duration_since(at));
            self.lap_at.set(Some(now));
        }
    }

    /// Closes the window when it is over; `placeholders` is the nanoseconds per placeholder.
    pub fn roll(&self, now_ms: i64, placeholders: HashMap<String, u64>) {
        let mut i = self.inner.borrow_mut();
        if !i.tracking || now_ms - i.window_start_ms < WINDOW_MS {
            return;
        }
        let window_ns = ((now_ms - i.window_start_ms) as f64) * 1_000_000.0;
        let mut report = Report::default();
        for ((feature, category), ns) in i.features.drain() {
            report
                .features
                .entry(feature)
                .or_default()
                .insert(category, ns as f64 / window_ns * 100.0);
        }
        report.placeholders = placeholders
            .into_iter()
            .map(|(p, ns)| (p, ns as f64 / window_ns * 100.0))
            .collect();
        report
            .placeholders
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
        i.last = Some(report);
        i.window_start_ms = now_ms;
    }

    pub fn last_report(&self) -> Option<Report> {
        self.inner.borrow().last.clone()
    }
}

/// Java's `DecimalFormat("#.###")`: up to three decimals, no trailing zeros.
pub fn decimal3(v: f64) -> String {
    let s = format!("{:.3}", (v * 1000.0).round() / 1000.0);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
    if s.is_empty() || s == "-0" {
        "0".to_owned()
    } else {
        s
    }
}

/// `CpuCommand.colorize`: red above the first threshold, yellow above the second, else green.
fn colorize(usage: &str, t1: f64, t2: f64) -> String {
    let p: f64 = usage.parse().unwrap_or(0.0);
    let c = if p > t1 {
        "&c"
    } else if p > t2 {
        "&e"
    } else {
        "&a"
    };
    format!("{c}{usage}")
}

const BAR: char = '\u{2551}';

/// The lines `/tab cpu` prints for a report. `refresh` gives a placeholder's refresh interval and
/// whether it is set in the config.
pub fn lines(report: &Report, console: bool, refresh: &dyn Fn(&str) -> (i64, bool)) -> Vec<String> {
    let mut out = vec![" ".to_owned()];
    out.push(format!("&8&l{BAR}&8&m             &r&8&l[ &bTAB CPU Stats &8&l]&r&8&l&m             "));
    out.push(format!("&8&l{BAR} &6CPU stats from the last 10 seconds"));
    let rule = format!("&8&l{BAR}&8&m                                                    ");
    out.push(rule.clone());
    out.push(format!("&8&l{BAR} &6Top 5 placeholders:"));
    for (name, usage) in report.placeholders.iter().take(5) {
        let (interval, configured) = refresh(name);
        let refresh_text = format!(" {} ({})&7", if configured { "&3" } else { "&8" }, interval);
        let shown = decimal3(*usage);
        let colored = if name.starts_with("%sync:") {
            format!("&c{shown}")
        } else {
            colorize(&shown, 1.0, 0.3)
        };
        out.push(format!("&8&l{BAR} &7{name}{refresh_text} - {colored}%"));
    }
    out.push(rule.clone());
    if console {
        out.push(format!("&8&l{BAR} &6Features:"));
        for (feature, cats) in &report.features {
            let total: f64 = cats.values().sum();
            out.push(format!(
                "&8&l{BAR} &7{feature} &7({}%&7):",
                colorize(&decimal3(total), 5.0, 1.0)
            ));
            for (cat, v) in cats {
                out.push(format!("&8&l{BAR}     &7{cat} - {}%", colorize(&decimal3(*v), 5.0, 1.0)));
            }
        }
    } else {
        out.push(format!("&8&l{BAR} &6Features (execute from console for more info):"));
        for (feature, cats) in &report.features {
            let total: f64 = cats.values().sum();
            out.push(format!(
                "&8&l{BAR} &7{feature} &7({}%&7):",
                colorize(&decimal3(total), 5.0, 1.0)
            ));
        }
    }
    out.push(rule);
    let (ph, all) = (report.placeholder_total(), report.feature_total());
    out.push(format!(
        "&8&l{BAR} &6&lPlaceholders Total: &a&l{}%",
        colorize(&decimal3(ph), 10.0, 5.0)
    ));
    out.push(format!(
        "&8&l{BAR} &6&lPlugin internals: &a&l{}%",
        colorize(&decimal3(all - ph), 10.0, 5.0)
    ));
    out.push(format!("&8&l{BAR} &6&lTotal: &e&l{}%", colorize(&decimal3(all), 10.0, 5.0)));
    out.push(format!("&8&l{BAR}&8&m             &r&8&l[ &bTAB CPU Stats &8&l]&r&8&l&m             "));
    out.push(" ".to_owned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_like_java() {
        assert_eq!(decimal3(0.0), "0");
        assert_eq!(decimal3(1.5), "1.5");
        assert_eq!(decimal3(0.12345), "0.123");
        assert_eq!(decimal3(12.0), "12");
    }

    #[test]
    fn a_window_becomes_percent() {
        let cpu = Cpu::default();
        assert!(cpu.enable(0));
        assert!(!cpu.enable(5));
        cpu.add("Scoreboard", "Updating", Duration::from_millis(500));
        cpu.add("Scoreboard", "Updating", Duration::from_millis(500));
        let mut ph = HashMap::new();
        ph.insert("%ping%".to_owned(), 100_000_000u64);
        cpu.roll(5_000, ph.clone());
        assert!(cpu.last_report().is_none(), "the window is 10 s");
        cpu.roll(10_000, ph);
        let r = cpu.last_report().unwrap();
        assert!((r.features["Scoreboard"]["Updating"] - 10.0).abs() < 1e-9);
        assert_eq!(r.placeholders, vec![("%ping%".to_owned(), 1.0)]);
        assert!((r.feature_total() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_is_counted_before_tracking() {
        let cpu = Cpu::default();
        cpu.add("X", "Y", Duration::from_secs(1));
        cpu.enable(0);
        cpu.roll(10_000, HashMap::new());
        assert!(cpu.last_report().unwrap().features.is_empty());
    }

    #[test]
    fn the_report_lines() {
        let mut r = Report::default();
        r.features.entry("Layout".into()).or_default().insert("Updating".into(), 2.0);
        r.placeholders.push(("%ping%".into(), 0.5));
        let l = lines(&r, false, &|_| (500, false));
        assert_eq!(l[0], " ");
        assert!(l.contains(&format!("&8&l{BAR} &7%ping% &8 (500)&7 - &e0.5%")));
        assert!(l.contains(&format!("&8&l{BAR} &7Layout &7(&e2%&7):")));
        assert!(l.contains(&format!("&8&l{BAR} &6&lTotal: &e&l&e2%")));
        let c = lines(&r, true, &|_| (500, true));
        assert!(c.contains(&format!("&8&l{BAR}     &7Updating - &e2%")));
    }
}
