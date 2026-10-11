//! Command-line grammar of `site-preview`, declared with `clap`'s derive API.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use preview_composition::{DebounceWindow, DebounceWindowError};

/// Zero-config local live-reload server for static sites.
#[derive(Debug, Parser)]
#[command(name = "site-preview", version, about, arg_required_else_help = true)]
pub struct Cli {
    /// The action to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands of `site-preview`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve a directory and reload the browser when a file changes.
    Serve(ServeArgs),
}

/// The arguments of `serve`. Options may come before or after the directory (plan.md
/// section 3), pinned by `should_accept_directory_when_given_after_flags` and
/// `should_accept_directory_when_given_before_flags`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// The directory to serve (the current directory when omitted).
    #[arg(default_value = ".")]
    pub directory: PathBuf,

    /// The address to listen on; outside loopback it needs --allow-remote.
    #[arg(long, value_name = "IP", default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    pub host: IpAddr,

    /// The port to listen on, exactly; 0 lets the system choose a free one.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub port: u16,

    /// Allow listening outside loopback. There is no authentication: anyone who can reach
    /// the address can read the site.
    #[arg(long)]
    pub allow_remote: bool,

    /// How long the site must stay quiet before the browser reloads: 10ms to 10s, written
    /// with an `ms` or `s` unit.
    #[arg(long, value_name = "DUR", default_value = "150ms", value_parser = parse_debounce)]
    pub debounce: DebounceWindow,

    /// Print start-up and watcher events as JSON lines on stdout.
    #[arg(long)]
    pub json: bool,

    /// Open the site in a browser once it is ready.
    #[arg(long)]
    pub open: bool,
}

/// Parses `--debounce`: a whole number with an `ms` or `s` unit, in the range of D22
/// (ADR 0023). Pinned by `should_parse_debounce_when_unit_is_ms_or_s` and
/// `should_refuse_debounce_when_malformed_or_out_of_range`.
fn parse_debounce(text: &str) -> Result<DebounceWindow, String> {
    let malformed =
        || format!("expected a duration such as 150ms or 1s (allowed: 10ms to 10s), got {text:?}");
    let (digits, scale) = if let Some(digits) = text.strip_suffix("ms") {
        (digits, 1)
    } else if let Some(digits) = text.strip_suffix('s') {
        (digits, 1_000)
    } else {
        return Err(malformed());
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    let millis = digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .ok_or_else(|| DebounceWindowError::TooLong.to_string())?;
    DebounceWindow::from_millis(millis).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serve(argv: &[&str]) -> ServeArgs {
        let cli = Cli::try_parse_from(argv).expect("serve parses");
        let Command::Serve(args) = cli.command;
        args
    }

    #[test]
    fn should_parse_debounce_when_unit_is_ms_or_s() {
        // given
        let cases = [("150ms", 150), ("10ms", 10), ("1s", 1_000), ("10s", 10_000)];

        for (text, millis) in cases {
            // when
            let window = parse_debounce(text);

            // then
            assert_eq!(window.map(DebounceWindow::as_millis), Ok(millis), "{text}");
        }
    }

    #[test]
    fn should_refuse_debounce_when_malformed_or_out_of_range() {
        // given
        let texts = [
            "0ms",
            "0s",
            "9ms",
            "11s",
            "10001ms",
            "150",
            "",
            "ms",
            "s",
            "1.5s",
            "-1s",
            "+5s",
            "1 s",
            "1m",
            "1h",
            "99999999999999999999ms",
            "18446744073709551615s",
            "1S",
        ];

        for text in texts {
            // when
            let window = parse_debounce(text);

            // then
            let message = window.expect_err(text);
            assert!(message.contains("10ms to 10s"), "{text}: {message}");
        }
    }

    #[test]
    fn should_default_every_serve_option_when_only_directory_given() {
        // given
        let argv = ["site-preview", "serve", "public"];

        // when
        let args = serve(&argv);

        // then
        assert_eq!(args.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(args.port, 0);
        assert!(!args.allow_remote);
        assert_eq!(args.debounce, DebounceWindow::DEFAULT);
        assert!(!args.json);
        assert!(!args.open);
    }

    #[test]
    fn should_parse_every_serve_option_when_given() {
        // given
        let argv = [
            "site-preview",
            "serve",
            "--host",
            "::1",
            "--port",
            "4173",
            "--allow-remote",
            "--debounce",
            "1s",
            "--json",
            "--open",
            "public",
        ];

        // when
        let args = serve(&argv);

        // then
        assert_eq!(args.directory, PathBuf::from("public"));
        assert_eq!(args.host, "::1".parse::<IpAddr>().expect("an address"));
        assert_eq!(args.port, 4173);
        assert!(args.allow_remote);
        assert_eq!(args.debounce.as_millis(), 1_000);
        assert!(args.json);
        assert!(args.open);
    }

    #[test]
    fn should_refuse_host_when_not_numeric_ip() {
        // given
        let hosts = ["localhost", "example.com", "127.1", "[::1]"];

        for host in hosts {
            // when
            let parsed = Cli::try_parse_from(["site-preview", "serve", "--host", host]);

            // then
            assert!(parsed.is_err(), "{host}");
        }
    }

    #[test]
    fn should_serve_current_directory_when_no_directory_is_given() {
        // given
        let argv = ["site-preview", "serve"];

        // when
        let cli = Cli::try_parse_from(argv).expect("serve parses");

        // then
        let Command::Serve(args) = cli.command;
        assert_eq!(args.directory, PathBuf::from("."));
    }

    #[test]
    fn should_take_directory_when_given_as_positional() {
        // given
        let argv = ["site-preview", "serve", "public"];

        // when
        let cli = Cli::try_parse_from(argv).expect("serve parses");

        // then
        let Command::Serve(args) = cli.command;
        assert_eq!(args.directory, PathBuf::from("public"));
    }
}
