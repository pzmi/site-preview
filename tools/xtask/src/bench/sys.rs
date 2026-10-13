//! Process measurements: resident memory, CPU time and open file descriptors of another
//! process. Linux reads `/proc/<pid>`; elsewhere (macOS) `ps` and `lsof` answer. The parsers
//! are pure so their formats are pinned by unit tests.

use std::path::Path;
use std::process::Command;

/// CPU time of the process in seconds, user plus system.
pub fn cpu_seconds(pid: u32) -> Result<f64, String> {
    if Path::new(&format!("/proc/{pid}/stat")).exists() {
        let stat = read(&format!("/proc/{pid}/stat"))?;
        let ticks = parse_proc_stat_cpu_ticks(&stat).ok_or("unparsable /proc/<pid>/stat")?;
        // Ticks are in units of CLK_TCK; the value is 100 on every Linux in use.
        Ok(ticks as f64 / clock_ticks_per_second())
    } else {
        let out = ps(pid, "cputime")?;
        parse_ps_cputime_secs(&out).ok_or_else(|| format!("unparsable ps cputime {out:?}"))
    }
}

/// Resident set size of the process in KiB.
pub fn rss_kib(pid: u32) -> Result<u64, String> {
    if Path::new(&format!("/proc/{pid}/status")).exists() {
        let status = read(&format!("/proc/{pid}/status"))?;
        parse_status_vmrss_kib(&status).ok_or_else(|| "no VmRSS in /proc/<pid>/status".to_owned())
    } else {
        let out = ps(pid, "rss")?;
        parse_ps_rss_kib(&out).ok_or_else(|| format!("unparsable ps rss {out:?}"))
    }
}

/// Number of open file descriptors of the process.
pub fn fd_count(pid: u32) -> Result<usize, String> {
    let proc_fd = format!("/proc/{pid}/fd");
    if Path::new(&proc_fd).exists() {
        return std::fs::read_dir(&proc_fd)
            .map(|entries| entries.count())
            .map_err(|e| format!("cannot list {proc_fd}: {e}"));
    }
    let out = Command::new("lsof")
        .args(["-n", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("cannot run lsof: {e}"))?;
    // One header line, then one line per descriptor.
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .count()
        .saturating_sub(1))
}

fn clock_ticks_per_second() -> f64 {
    Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<f64>()
                .ok()
        })
        .filter(|ticks| *ticks > 0.0)
        .unwrap_or(100.0)
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
}

fn ps(pid: u32, column: &str) -> Result<String, String> {
    let out = Command::new("ps")
        .args(["-o", &format!("{column}="), "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("cannot run ps: {e}"))?;
    if !out.status.success() {
        return Err(format!("ps exited {} for pid {pid}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `utime + stime` in clock ticks from the text of `/proc/<pid>/stat`. The command name in
/// field 2 is parenthesised and may hold spaces and parentheses, so fields are counted after
/// the last `)`.
pub fn parse_proc_stat_cpu_ticks(stat: &str) -> Option<u64> {
    let after = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = after.split_whitespace().collect();
    // `after` starts at field 3 (state); utime and stime are fields 14 and 15.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// The `VmRSS` value in KiB from the text of `/proc/<pid>/status`.
pub fn parse_status_vmrss_kib(status: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.trim_start_matches("VmRSS:")
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// The output of `ps -o rss=`: KiB.
pub fn parse_ps_rss_kib(out: &str) -> Option<u64> {
    out.trim().parse().ok()
}

/// The output of `ps -o cputime=`: `[[dd-]hh:]mm:ss[.ff]`, in seconds.
pub fn parse_ps_cputime_secs(out: &str) -> Option<f64> {
    let text = out.trim();
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<f64>().ok()?, clock),
        None => (0.0, text),
    };
    let mut seconds = 0.0;
    for part in clock.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86_400.0 + seconds)
}

/// The first `model name` of the text of `/proc/cpuinfo`.
pub fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    cpuinfo
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, model)| model.trim().to_owned())
        .filter(|model| !model.is_empty())
}

/// The three load averages of the text of `/proc/loadavg`, as `"1m 5m 15m"`.
pub fn parse_loadavg(text: &str) -> Option<String> {
    let fields: Vec<&str> = text.split_whitespace().take(3).collect();
    (fields.len() == 3 && fields.iter().all(|f| f.parse::<f64>().is_ok())).then(|| fields.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "4242 (site-preview) S 1 4242 4242 0 -1 4194560 100 0 0 0 \
                        120 30 0 0 20 0 5 0 99999 12345678 2000 18446744073709551615";

    #[test]
    fn should_sum_user_and_system_ticks_when_stat_is_parsed() {
        // given
        let stat = STAT;

        // when
        let ticks = parse_proc_stat_cpu_ticks(stat);

        // then
        assert_eq!(ticks, Some(150));
    }

    #[test]
    fn should_count_after_last_paren_when_command_name_has_spaces() {
        // given
        let stat = STAT.replace("(site-preview)", "(site (pre) view)");

        // when
        let ticks = parse_proc_stat_cpu_ticks(&stat);

        // then
        assert_eq!(ticks, Some(150));
    }

    #[test]
    fn should_return_none_when_stat_is_truncated() {
        // given
        let stat = "4242 (x) S 1 2 3";

        // when
        let ticks = parse_proc_stat_cpu_ticks(stat);

        // then
        assert_eq!(ticks, None);
    }

    #[test]
    fn should_read_vmrss_when_status_is_parsed() {
        // given
        let status = "Name:\tsite-preview\nVmPeak:\t  90000 kB\nVmRSS:\t   12345 kB\nThreads:\t5\n";

        // when
        let rss = parse_status_vmrss_kib(status);

        // then
        assert_eq!(rss, Some(12345));
    }

    #[test]
    fn should_return_none_when_status_has_no_vmrss() {
        // given
        let status = "Name:\tzombie\nState:\tZ\n";

        // when
        let rss = parse_status_vmrss_kib(status);

        // then
        assert_eq!(rss, None);
    }

    #[test]
    fn should_parse_kib_when_ps_prints_rss() {
        // given
        let out = "  8192\n";

        // when
        let rss = parse_ps_rss_kib(out);

        // then
        assert_eq!(rss, Some(8192));
    }

    #[test]
    fn should_parse_all_forms_when_ps_prints_cputime() {
        // given
        let forms = ["0:01.50", "1:02:03", "2-00:00:10", "00:00:00.25", "garbage"];

        // when
        let parsed: Vec<Option<f64>> = forms.iter().map(|f| parse_ps_cputime_secs(f)).collect();

        // then
        assert_eq!(
            parsed,
            [Some(1.5), Some(3723.0), Some(172_810.0), Some(0.25), None]
        );
    }

    #[test]
    fn should_find_first_model_when_cpuinfo_lists_cores() {
        // given
        let info = "processor\t: 0\nmodel name\t: Example CPU @ 3.00GHz\nprocessor\t: 1\n\
                    model name\t: Other\n";

        // when
        let model = parse_cpu_model(info);

        // then
        assert_eq!(model.as_deref(), Some("Example CPU @ 3.00GHz"));
    }

    #[test]
    fn should_return_none_when_cpuinfo_has_no_model() {
        // given
        let info = "processor\t: 0\nBogoMIPS\t: 50.00\n";

        // when
        let model = parse_cpu_model(info);

        // then
        assert_eq!(model, None);
    }

    #[test]
    fn should_keep_three_numbers_when_loadavg_is_parsed() {
        // given
        let valid = "0.52 0.41 0.30 2/310 9999\n";
        let invalid = "a b c";

        // when
        let results = (parse_loadavg(valid), parse_loadavg(invalid));

        // then
        assert_eq!(results, (Some("0.52 0.41 0.30".to_owned()), None));
    }

    #[test]
    fn should_measure_own_process_when_samplers_run() {
        // given
        let pid = std::process::id();

        // when
        let rss = rss_kib(pid).unwrap();
        let fds = fd_count(pid).unwrap();
        let cpu = cpu_seconds(pid).unwrap();

        // then
        assert!(rss > 0);
        assert!(fds >= 3);
        assert!(cpu >= 0.0);
    }
}
