//! The benchmark report: what was measured, against which target, and the JSON that
//! `target/bench/report.json` holds. Written by hand like the catalog (no JSON dependency).

use crate::catalog::json_str;

/// One measured value against its plan section 13 target.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    /// Stable machine name.
    pub name: String,
    /// The measured value.
    pub value: f64,
    /// Unit of `value`.
    pub unit: &'static str,
    /// The target in words, as written in plan section 13 or the threshold chosen for it.
    pub target: String,
    /// Whether `value` is within the target.
    pub met: bool,
}

impl Metric {
    /// A metric that meets its target when `value <= limit`.
    pub fn at_most(name: &str, value: f64, unit: &'static str, limit: f64, target: &str) -> Self {
        Self {
            name: name.to_owned(),
            value,
            unit,
            target: target.to_owned(),
            met: value <= limit,
        }
    }
}

/// A pass/fail assertion of the soak (no leak, stable memory, the connection cap).
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// Stable machine name.
    pub name: String,
    /// Whether the assertion held.
    pub passed: bool,
    /// The numbers behind the verdict.
    pub detail: String,
}

/// Everything one `cargo xtask bench` run measured.
#[derive(Debug, Default)]
pub struct Report {
    /// Where and how it ran, as key and value.
    pub environment: Vec<(String, String)>,
    /// Measured values against targets.
    pub metrics: Vec<Metric>,
    /// Soak assertions.
    pub checks: Vec<Check>,
    /// Raw samples by name, in milliseconds.
    pub samples: Vec<(String, Vec<f64>)>,
}

impl Report {
    /// True when every soak assertion held. A missed timing target is reported, not failed.
    pub fn checks_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// The metrics that missed their target.
    pub fn misses(&self) -> Vec<&Metric> {
        self.metrics.iter().filter(|m| !m.met).collect()
    }

    /// Renders the report as JSON.
    pub fn to_json(&self) -> String {
        let environment = self
            .environment
            .iter()
            .map(|(k, v)| format!("    {}: {}", json_str(k), json_str(v)))
            .collect::<Vec<_>>()
            .join(",\n");
        let metrics = self
            .metrics
            .iter()
            .map(|m| {
                format!(
                    "    {{\"name\": {}, \"value\": {}, \"unit\": {}, \"target\": {}, \"met\": {}}}",
                    json_str(&m.name),
                    number(m.value),
                    json_str(m.unit),
                    json_str(&m.target),
                    m.met
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        let checks = self
            .checks
            .iter()
            .map(|c| {
                format!(
                    "    {{\"name\": {}, \"passed\": {}, \"detail\": {}}}",
                    json_str(&c.name),
                    c.passed,
                    json_str(&c.detail)
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        let samples = self
            .samples
            .iter()
            .map(|(name, values)| {
                let values: Vec<String> = values.iter().map(|v| number(*v)).collect();
                format!("    {}: [{}]", json_str(name), values.join(", "))
            })
            .collect::<Vec<_>>()
            .join(",\n");
        format!(
            "{{\n  \"schema\": 1,\n  \"environment\": {{\n{environment}\n  }},\n  \
             \"metrics\": [\n{metrics}\n  ],\n  \"checks\": [\n{checks}\n  ],\n  \
             \"samples_ms\": {{\n{samples}\n  }}\n}}\n"
        )
    }
}

fn number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.2}")
    } else {
        "null".to_owned()
    }
}

/// Idle CPU as a percentage of one core: `cpu_seconds` consumed over `wall_seconds`.
pub fn cpu_percent(cpu_seconds: f64, wall_seconds: f64) -> f64 {
    if wall_seconds <= 0.0 {
        return 0.0;
    }
    cpu_seconds / wall_seconds * 100.0
}

/// Whether resident memory stayed stable between the warmed-up state and the end of the soak:
/// growth of at most the larger of a quarter of the warm size and 4 MiB. The slack absorbs
/// allocator noise. Measured from save 20, the 180 later saves fill it with a leak of about
/// 22 KiB per save (4 MiB over 180); a smaller per-save leak, such as one small buffer, passes
/// undetected, and so does one that has already reached its steady state by save 20. The
/// descriptor check, not this one, catches a leaked connection.
pub fn rss_is_stable(warm_kib: u64, end_kib: u64) -> bool {
    end_kib <= warm_kib + (warm_kib / 4).max(4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report {
            environment: vec![("os".to_owned(), "linux \"x\"".to_owned())],
            metrics: vec![
                Metric::at_most("startup_median", 12.345, "ms", 1000.0, "< 1000 ms"),
                Metric::at_most("latency_p95", 612.0, "ms", 500.0, "<= 500 ms"),
            ],
            checks: vec![Check {
                name: "fds".to_owned(),
                passed: true,
                detail: "12 -> 12".to_owned(),
            }],
            samples: vec![("startup".to_owned(), vec![1.0, 2.5])],
        }
    }

    #[test]
    fn should_render_every_section_when_report_is_serialised() {
        // given
        let report = sample();

        // when
        let json = report.to_json();

        // then
        assert!(json.starts_with("{\n  \"schema\": 1,"), "{json}");
        assert!(json.contains(r#""os": "linux \"x\"""#), "{json}");
        assert!(
            json.contains(
                r#"{"name": "startup_median", "value": 12.35, "unit": "ms", "target": "< 1000 ms", "met": true}"#
            ),
            "{json}"
        );
        assert!(json.contains(r#""name": "latency_p95""#) && json.contains(r#""met": false"#));
        assert!(
            json.contains(r#"{"name": "fds", "passed": true, "detail": "12 -> 12"}"#),
            "{json}"
        );
        assert!(json.contains(r#""startup": [1.00, 2.50]"#), "{json}");
        assert!(json.ends_with("}\n"));
    }

    #[test]
    fn should_write_null_when_value_is_not_finite() {
        // given
        let value = f64::NAN;

        // when
        let text = number(value);

        // then
        assert_eq!(text, "null");
    }

    #[test]
    fn should_list_only_missed_targets_when_misses_are_asked() {
        // given
        let report = sample();

        // when
        let misses: Vec<&str> = report.misses().iter().map(|m| m.name.as_str()).collect();

        // then
        assert_eq!(misses, ["latency_p95"]);
        assert!(report.checks_passed());
    }

    #[test]
    fn should_fail_checks_when_one_assertion_failed() {
        // given
        let mut report = sample();
        report.checks.push(Check {
            name: "rss".to_owned(),
            passed: false,
            detail: String::new(),
        });

        // when
        let passed = report.checks_passed();

        // then
        assert!(!passed);
    }

    #[test]
    fn should_meet_target_when_value_equals_limit() {
        // given
        let metric = Metric::at_most("m", 500.0, "ms", 500.0, "t");

        // when
        let met = metric.met;

        // then
        assert!(met);
    }

    #[test]
    fn should_convert_cpu_seconds_to_percent_when_wall_time_is_known() {
        // given
        let (cpu, wall) = (0.3, 30.0);

        // when
        let percent = cpu_percent(cpu, wall);

        // then
        assert!((percent - 1.0).abs() < 1e-9);
        assert!(cpu_percent(1.0, 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn should_judge_growth_when_rss_is_compared() {
        // given
        let warm = 100_000;

        // when
        let within = rss_is_stable(warm, 125_000);
        let beyond = rss_is_stable(warm, 125_001);
        let shrunk = rss_is_stable(warm, 80_000);
        let small_warm = rss_is_stable(1_000, 5_096);

        // then
        assert!(within);
        assert!(!beyond);
        assert!(shrunk);
        assert!(small_warm, "the 4 MiB floor applies to a small process");
    }
}
