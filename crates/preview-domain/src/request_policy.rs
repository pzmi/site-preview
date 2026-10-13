//! Host authority and SSE Origin policy (plan.md §5.4, ADR 0021, ADR 0037).
//!
//! Both checks are pure: the HTTP adapter hands in the raw header values and turns the
//! decision into a status code. A request first passes [`HostPolicy::check`]; the internal
//! SSE route then also passes [`OriginPolicy::check`] against the authority the Host check
//! accepted.
//!
//! Addresses are `core::net` value types (ADR 0037): they parse and compare, and own no
//! socket.

use core::fmt;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The port an `http` authority without an explicit port refers to.
const DEFAULT_HTTP_PORT: u16 = 80;

/// The address and port the listener is bound to.
///
/// Loopback and wildcard are derived from the address, so they cannot disagree with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundAddr {
    ip: IpAddr,
    port: u16,
}

impl BoundAddr {
    /// The listener bound to `ip` and `port` (the port actually bound, never 0).
    #[must_use]
    pub const fn new(ip: IpAddr, port: u16) -> Self {
        Self { ip, port }
    }

    /// The bound address.
    #[must_use]
    pub const fn ip(&self) -> IpAddr {
        self.ip
    }

    /// The bound port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Whether only this machine can reach the listener (`127.0.0.0/8`, `::1`, or an
    /// IPv4-mapped loopback address).
    #[must_use]
    pub const fn is_loopback(&self) -> bool {
        self.ip.to_canonical().is_loopback()
    }

    /// Whether the listener accepts on every interface (`0.0.0.0` or `::`).
    #[must_use]
    pub const fn is_wildcard(&self) -> bool {
        self.ip.to_canonical().is_unspecified()
    }
}

/// The host part of an accepted authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityHost {
    /// The name `localhost`, matched without regard to ASCII case.
    Localhost,
    /// A numeric address: dotted-quad IPv4, or bracketed IPv6 without a zone id.
    Ip(IpAddr),
}

/// A Host authority the policy accepted, with the port made explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authority {
    host: AuthorityHost,
    port: u16,
}

impl Authority {
    /// The host part.
    #[must_use]
    pub const fn host(&self) -> AuthorityHost {
        self.host
    }

    /// The port, 80 when the header named none.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

/// Why a Host header was refused. The HTTP adapter answers each with 421.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostReject {
    /// No Host header (HTTP/1.1 requires one).
    Missing,
    /// Not `host[:port]`: empty, whitespace or non-ASCII, a bad port, a bare IPv6
    /// address, an unclosed bracket or trailing text after it.
    Malformed,
    /// Contains `@`: userinfo has no place in a Host header.
    Userinfo,
    /// A bracketed IPv6 address with a zone id (`%`), never sent by a browser.
    ZoneId,
    /// A DNS name other than `localhost`: the DNS-rebinding case.
    DnsName,
    /// `localhost` while the listener is not loopback.
    LocalhostNotAllowed,
    /// `0.0.0.0` or `::`: a listener's wildcard, never a destination.
    UnspecifiedAddress,
    /// A numeric address this listener does not answer for.
    AddressNotAllowed,
    /// The port differs from the bound port.
    PortMismatch,
}

impl HostReject {
    /// A stable, lowercase code for logs and the refusal body.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "host_missing",
            Self::Malformed => "host_malformed",
            Self::Userinfo => "host_userinfo",
            Self::ZoneId => "host_zone_id",
            Self::DnsName => "host_dns_name",
            Self::LocalhostNotAllowed => "host_localhost_not_allowed",
            Self::UnspecifiedAddress => "host_unspecified_address",
            Self::AddressNotAllowed => "host_address_not_allowed",
            Self::PortMismatch => "host_port_mismatch",
        }
    }
}

impl fmt::Display for HostReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The outcome of [`HostPolicy::check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum HostDecision {
    /// The request may proceed; the authority is what the Origin check compares against.
    Allow(Authority),
    /// The request is refused.
    Reject(HostReject),
}

/// Which Host authorities a listener answers for (ADR 0021, ADR 0037).
///
/// - The bound numeric address with the bound port, always
///   (`should_allow_bound_ip_when_port_matches`).
/// - `localhost:<port>` only when the listener is loopback
///   (`should_allow_localhost_alias_when_bound_to_loopback`,
///   `should_reject_localhost_when_allow_remote_on_wildcard`).
/// - With `allow_remote` on a wildcard listener, any numeric address but the unspecified
///   one (`should_allow_numeric_ip_when_wildcard_and_allow_remote`).
/// - Never another DNS name (`should_reject_when_host_is_dns_name`), and never a different
///   port (`should_reject_when_port_mismatches`).
///
/// A wildcard listener without `allow_remote` answers for nothing: the CLI refuses that
/// configuration, and the policy fails closed if it is reached anyway
/// (`should_reject_everything_when_wildcard_without_allow_remote`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPolicy {
    bound: BoundAddr,
    allow_remote: bool,
}

impl HostPolicy {
    /// The policy for a listener at `bound`, in `--allow-remote` mode or not.
    #[must_use]
    pub const fn new(bound: BoundAddr, allow_remote: bool) -> Self {
        Self {
            bound,
            allow_remote,
        }
    }

    /// Decides a request's Host header value; `None` when the request has none.
    pub fn check(&self, host_header: Option<&str>) -> HostDecision {
        let Some(raw) = host_header else {
            return HostDecision::Reject(HostReject::Missing);
        };
        match self.authorize(raw) {
            Ok(authority) => HostDecision::Allow(authority),
            Err(reason) => HostDecision::Reject(reason),
        }
    }

    fn authorize(&self, raw: &str) -> Result<Authority, HostReject> {
        let (parsed, port) = parse_authority(raw)?;
        let host = match parsed {
            ParsedHost::Name => return Err(HostReject::DnsName),
            ParsedHost::Localhost if !self.bound.is_loopback() => {
                return Err(HostReject::LocalhostNotAllowed);
            }
            ParsedHost::Localhost => AuthorityHost::Localhost,
            ParsedHost::Ip(ip) if ip.to_canonical().is_unspecified() => {
                return Err(HostReject::UnspecifiedAddress);
            }
            ParsedHost::Ip(ip) if ip == self.bound.ip => AuthorityHost::Ip(ip),
            ParsedHost::Ip(ip) if self.allow_remote && self.bound.is_wildcard() => {
                AuthorityHost::Ip(ip)
            }
            ParsedHost::Ip(_) => return Err(HostReject::AddressNotAllowed),
        };
        if port != self.bound.port {
            return Err(HostReject::PortMismatch);
        }
        Ok(Authority { host, port })
    }
}

/// Why the SSE endpoint refused a browser request. The HTTP adapter answers each with 403.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginReject {
    /// `Sec-Fetch-Site: cross-site`.
    CrossSite,
    /// `Origin: null`: a sandboxed, `file:` or otherwise opaque origin.
    Opaque,
    /// A scheme other than `http`; this server serves plain HTTP only.
    SchemeNotHttp,
    /// Not `http://host[:port]`: a path, userinfo, zone id or unparsable authority.
    Malformed,
    /// A well-formed origin whose authority differs from the request's Host authority.
    AuthorityMismatch,
}

impl OriginReject {
    /// A stable, lowercase code for logs and the refusal body.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::CrossSite => "origin_cross_site",
            Self::Opaque => "origin_opaque",
            Self::SchemeNotHttp => "origin_scheme_not_http",
            Self::Malformed => "origin_malformed",
            Self::AuthorityMismatch => "origin_authority_mismatch",
        }
    }
}

impl fmt::Display for OriginReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The outcome of [`OriginPolicy::check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum OriginDecision {
    /// The SSE connection may open.
    Allow,
    /// The SSE connection is refused.
    Reject(OriginReject),
}

/// Browser-header policy of the internal SSE endpoint (plan.md §5.4, ADR 0037).
///
/// - `Sec-Fetch-Site: cross-site` is refused (`should_reject_when_origin_is_cross_site`);
///   `same-origin`, `same-site`, `none` and unknown values defer to the Origin check, since
///   only a browser sets the header and `same-site` still covers another local port
///   (`should_reject_when_origin_conflicts_with_host`).
/// - An `Origin` must be `http://` plus the very authority the Host check accepted, port
///   included; `localhost` and `127.0.0.1` are different origins
///   (`should_reject_when_origin_conflicts_with_host`), and `null` is refused
///   (`should_reject_when_origin_is_null`).
/// - A request with neither header is allowed: agents and `curl` send none
///   (`should_allow_sse_when_no_browser_headers`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginPolicy;

impl OriginPolicy {
    /// Decides an SSE request's `Origin` and `Sec-Fetch-Site` values against `host`, the
    /// authority [`HostPolicy::check`] accepted for the same request.
    pub fn check(
        origin: Option<&str>,
        sec_fetch_site: Option<&str>,
        host: &Authority,
    ) -> OriginDecision {
        if sec_fetch_site.is_some_and(|site| site.trim().eq_ignore_ascii_case("cross-site")) {
            return OriginDecision::Reject(OriginReject::CrossSite);
        }
        match origin.map(|value| origin_authority(value, host)) {
            None | Some(Ok(())) => OriginDecision::Allow,
            Some(Err(reason)) => OriginDecision::Reject(reason),
        }
    }
}

fn origin_authority(origin: &str, host: &Authority) -> Result<(), OriginReject> {
    if origin.eq_ignore_ascii_case("null") {
        return Err(OriginReject::Opaque);
    }
    let Some((scheme, rest)) = origin.split_once("://") else {
        return Err(OriginReject::Malformed);
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(OriginReject::SchemeNotHttp);
    }
    if rest.contains(['/', '?', '#']) {
        return Err(OriginReject::Malformed);
    }
    let (parsed, port) = parse_authority(rest).map_err(|_| OriginReject::Malformed)?;
    let matches = port == host.port
        && match (parsed, host.host) {
            (ParsedHost::Localhost, AuthorityHost::Localhost) => true,
            (ParsedHost::Ip(origin_ip), AuthorityHost::Ip(host_ip)) => origin_ip == host_ip,
            _ => false,
        };
    if matches {
        Ok(())
    } else {
        Err(OriginReject::AuthorityMismatch)
    }
}

/// A syntactically valid authority's host, before the listener's rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedHost {
    Localhost,
    Ip(IpAddr),
    /// Any other reg-name, including `localhost.` and `127.0.0.1.` (trailing dot) and
    /// shorthand such as `127.1`, which `Ipv4Addr` rejects.
    Name,
}

/// Parses `host[:port]` (RFC 9110 §7.2) with the port defaulting to 80.
///
/// Strict on purpose: printable ASCII only, a canonical decimal port, IPv6 only in
/// brackets and without a zone id, no userinfo.
fn parse_authority(raw: &str) -> Result<(ParsedHost, u16), HostReject> {
    if raw.contains('@') {
        return Err(HostReject::Userinfo);
    }
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(HostReject::Malformed);
    }
    if let Some(bracketed) = raw.strip_prefix('[') {
        let (inner, after) = bracketed.split_once(']').ok_or(HostReject::Malformed)?;
        if inner.contains('%') {
            return Err(HostReject::ZoneId);
        }
        let ip = inner
            .parse::<Ipv6Addr>()
            .map_err(|_| HostReject::Malformed)?;
        let port = match after {
            "" => DEFAULT_HTTP_PORT,
            _ => parse_port(after.strip_prefix(':').ok_or(HostReject::Malformed)?)?,
        };
        return Ok((ParsedHost::Ip(IpAddr::V6(ip)), port));
    }
    let (name, port) = match raw.split_once(':') {
        None => (raw, DEFAULT_HTTP_PORT),
        Some((name, port)) => (name, parse_port(port)?),
    };
    if name.is_empty() || name.contains(['[', ']']) {
        return Err(HostReject::Malformed);
    }
    let host = if let Ok(ip) = name.parse::<Ipv4Addr>() {
        ParsedHost::Ip(IpAddr::V4(ip))
    } else if name.eq_ignore_ascii_case("localhost") {
        ParsedHost::Localhost
    } else {
        ParsedHost::Name
    };
    Ok((host, port))
}

/// A canonical decimal port: 1–5 digits, no sign, no leading zero, at most 65535.
fn parse_port(digits: &str) -> Result<u16, HostReject> {
    let canonical = !digits.is_empty()
        && digits.len() <= 5
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && !(digits.len() > 1 && digits.starts_with('0'));
    if !canonical {
        return Err(HostReject::Malformed);
    }
    digits.parse::<u16>().map_err(|_| HostReject::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 8080;

    fn loopback_v4() -> HostPolicy {
        HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), PORT), false)
    }

    fn wildcard_remote() -> HostPolicy {
        HostPolicy::new(
            BoundAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT),
            true,
        )
    }

    fn ip_authority(ip: IpAddr) -> Authority {
        Authority {
            host: AuthorityHost::Ip(ip),
            port: PORT,
        }
    }

    fn rejected(policy: &HostPolicy, host: &str) -> Option<HostReject> {
        match policy.check(Some(host)) {
            HostDecision::Allow(_) => None,
            HostDecision::Reject(reason) => Some(reason),
        }
    }

    #[test]
    fn should_reject_when_host_is_dns_name() {
        // given
        let policy = loopback_v4();
        let host = format!("evil.example:{PORT}");

        // when
        let decision = policy.check(Some(&host));

        // then
        assert_eq!(decision, HostDecision::Reject(HostReject::DnsName));
    }

    #[test]
    fn should_reject_when_port_mismatches() {
        // given
        let policy = loopback_v4();

        // when
        let decisions = [
            policy.check(Some("127.0.0.1:8081")),
            policy.check(Some("127.0.0.1")),
            policy.check(Some("localhost:80")),
        ];

        // then
        assert_eq!(
            decisions,
            [HostDecision::Reject(HostReject::PortMismatch); 3]
        );
    }

    #[test]
    fn should_allow_bound_ip_when_port_matches() {
        // given
        let policy = loopback_v4();

        // when
        let decision = policy.check(Some("127.0.0.1:8080"));

        // then
        assert_eq!(
            decision,
            HostDecision::Allow(ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST)))
        );
    }

    #[test]
    fn should_allow_default_port_when_bound_to_port_eighty() {
        // given
        let policy = HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false);

        // when
        let decision = policy.check(Some("127.0.0.1"));

        // then
        assert!(matches!(decision, HostDecision::Allow(a) if a.port() == 80));
    }

    #[test]
    fn should_allow_localhost_alias_when_bound_to_loopback() {
        // given
        let v4 = loopback_v4();
        let v6 = HostPolicy::new(BoundAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), PORT), false);
        let loopback_with_flag =
            HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), PORT), true);
        let expected = HostDecision::Allow(Authority {
            host: AuthorityHost::Localhost,
            port: PORT,
        });

        // when
        let decisions = [
            v4.check(Some("localhost:8080")),
            v4.check(Some("LocalHost:8080")),
            v6.check(Some("localhost:8080")),
            loopback_with_flag.check(Some("localhost:8080")),
        ];

        // then
        assert_eq!(decisions, [expected; 4]);
    }

    #[test]
    fn should_reject_localhost_when_allow_remote_on_wildcard() {
        // given
        let policy = wildcard_remote();

        // when
        let decision = policy.check(Some("localhost:8080"));

        // then
        assert_eq!(
            decision,
            HostDecision::Reject(HostReject::LocalhostNotAllowed)
        );
    }

    #[test]
    fn should_reject_localhost_when_bound_to_specific_lan_address() {
        // given
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5));
        let policy = HostPolicy::new(BoundAddr::new(lan, PORT), true);

        // when
        let decisions = [
            policy.check(Some("localhost:8080")),
            policy.check(Some("192.168.1.5:8080")),
            policy.check(Some("192.168.1.6:8080")),
        ];

        // then
        assert_eq!(
            decisions,
            [
                HostDecision::Reject(HostReject::LocalhostNotAllowed),
                HostDecision::Allow(ip_authority(lan)),
                HostDecision::Reject(HostReject::AddressNotAllowed),
            ]
        );
    }

    #[test]
    fn should_allow_numeric_ip_when_wildcard_and_allow_remote() {
        // given
        let v4 = wildcard_remote();
        let v6 = HostPolicy::new(
            BoundAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), PORT),
            true,
        );
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5));
        let lan6: IpAddr = "fe80::1".parse().expect("valid literal");

        // when
        let decisions = [
            v4.check(Some("192.168.1.5:8080")),
            v6.check(Some("192.168.1.5:8080")),
            v6.check(Some("[fe80::1]:8080")),
            v4.check(Some("[FE80::1]:8080")),
        ];

        // then
        assert_eq!(
            decisions,
            [
                HostDecision::Allow(ip_authority(lan)),
                HostDecision::Allow(ip_authority(lan)),
                HostDecision::Allow(ip_authority(lan6)),
                HostDecision::Allow(ip_authority(lan6)),
            ]
        );
    }

    #[test]
    fn should_reject_dns_name_when_wildcard_and_allow_remote() {
        // given
        let policy = wildcard_remote();

        // when
        let reasons = [
            rejected(&policy, "evil.example:8080"),
            rejected(&policy, "my-laptop.local:8080"),
            rejected(&policy, "foo.localhost:8080"),
            rejected(&policy, "7f000001.rebind.example:8080"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::DnsName); 4]);
    }

    #[test]
    fn should_reject_everything_when_wildcard_without_allow_remote() {
        // given
        let policy = HostPolicy::new(
            BoundAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT),
            false,
        );

        // when
        let reasons = [
            rejected(&policy, "0.0.0.0:8080"),
            rejected(&policy, "127.0.0.1:8080"),
            rejected(&policy, "localhost:8080"),
        ];

        // then
        assert_eq!(
            reasons,
            [
                Some(HostReject::UnspecifiedAddress),
                Some(HostReject::AddressNotAllowed),
                Some(HostReject::LocalhostNotAllowed),
            ]
        );
    }

    #[test]
    fn should_reject_unspecified_host_when_wildcard_and_allow_remote() {
        // given
        let policy = wildcard_remote();

        // when
        let reasons = [
            rejected(&policy, "0.0.0.0:8080"),
            rejected(&policy, "[::]:8080"),
            rejected(&policy, "[::ffff:0.0.0.0]:8080"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::UnspecifiedAddress); 3]);
    }

    #[test]
    fn should_reject_when_host_header_is_missing() {
        // given
        let policy = loopback_v4();

        // when
        let decision = policy.check(None);

        // then
        assert_eq!(decision, HostDecision::Reject(HostReject::Missing));
    }

    #[test]
    fn should_reject_userinfo_when_host_carries_credentials() {
        // given
        let policy = loopback_v4();

        // when
        let reasons = [
            rejected(&policy, "user@127.0.0.1:8080"),
            rejected(&policy, "user:pass@127.0.0.1:8080"),
            rejected(&policy, "127.0.0.1:8080@evil.example"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::Userinfo); 3]);
    }

    #[test]
    fn should_reject_trailing_dot_when_name_or_address_ends_with_one() {
        // given
        let policy = loopback_v4();

        // when
        let reasons = [
            rejected(&policy, "localhost.:8080"),
            rejected(&policy, "127.0.0.1.:8080"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::DnsName); 2]);
    }

    #[test]
    fn should_reject_non_canonical_ipv4_when_shorthand_or_padded() {
        // given
        let policy = loopback_v4();

        // when
        let reasons = [
            rejected(&policy, "127.1:8080"),
            rejected(&policy, "2130706433:8080"),
            rejected(&policy, "0x7f.0.0.1:8080"),
            rejected(&policy, "127.000.000.001:8080"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::DnsName); 4]);
    }

    #[test]
    fn should_reject_ipv6_when_zone_id_present() {
        // given
        let policy = HostPolicy::new(BoundAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), PORT), false);

        // when
        let reasons = [
            rejected(&policy, "[::1%lo]:8080"),
            rejected(&policy, "[::1%25lo]:8080"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::ZoneId); 2]);
    }

    #[test]
    fn should_allow_bracketed_ipv6_when_bound_to_ipv6_loopback() {
        // given
        let policy = HostPolicy::new(BoundAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), PORT), false);

        // when
        let decisions = [
            policy.check(Some("[::1]:8080")),
            policy.check(Some("[0:0:0:0:0:0:0:1]:8080")),
        ];

        // then
        assert_eq!(
            decisions,
            [HostDecision::Allow(ip_authority(IpAddr::V6(Ipv6Addr::LOCALHOST))); 2]
        );
    }

    #[test]
    fn should_reject_other_address_when_bound_to_loopback() {
        // given
        let policy = loopback_v4();

        // when
        let reasons = [
            rejected(&policy, "127.0.0.2:8080"),
            rejected(&policy, "[::1]:8080"),
            rejected(&policy, "[::ffff:127.0.0.1]:8080"),
            rejected(&policy, "192.168.1.5:8080"),
        ];

        // then
        assert_eq!(reasons, [Some(HostReject::AddressNotAllowed); 4]);
    }

    #[test]
    fn should_reject_malformed_when_authority_syntax_is_invalid() {
        // given
        let policy = loopback_v4();
        let inputs = [
            "",
            ":8080",
            "127.0.0.1:",
            "127.0.0.1:08080",
            "127.0.0.1:+8080",
            "127.0.0.1:65536",
            "127.0.0.1:123456",
            "127.0.0.1:8080:8080",
            "::1",
            "::1:8080",
            "[::1",
            "[::1]8080",
            "[::1]:",
            "[127.0.0.1]:8080",
            "[v1.x]:8080",
            "127.0.0.1 :8080",
            " 127.0.0.1:8080",
            "127.0.0.1:8080\r\n",
            "l\u{f6}calhost:8080",
            "a]:8080",
        ];

        // when
        let reasons: Vec<_> = inputs.iter().map(|host| rejected(&policy, host)).collect();

        // then
        assert_eq!(reasons, vec![Some(HostReject::Malformed); inputs.len()]);
    }

    #[test]
    fn should_report_stable_codes_when_displaying_reasons() {
        // given
        let host = HostReject::PortMismatch;
        let origin = OriginReject::CrossSite;

        // when
        let shown = (host.to_string(), origin.to_string());

        // then
        assert_eq!(
            shown,
            (
                "host_port_mismatch".to_owned(),
                "origin_cross_site".to_owned()
            )
        );
    }

    #[test]
    fn should_reject_when_origin_is_cross_site() {
        // given
        let host = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));

        // when
        let decisions = [
            OriginPolicy::check(None, Some("cross-site"), &host),
            OriginPolicy::check(Some("http://127.0.0.1:8080"), Some("Cross-Site"), &host),
        ];

        // then
        assert_eq!(
            decisions,
            [OriginDecision::Reject(OriginReject::CrossSite); 2]
        );
    }

    #[test]
    fn should_allow_sse_when_no_browser_headers() {
        // given
        let host = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));

        // when
        let decision = OriginPolicy::check(None, None, &host);

        // then
        assert_eq!(decision, OriginDecision::Allow);
    }

    #[test]
    fn should_allow_sse_when_origin_matches_host() {
        // given
        let ip = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let localhost = Authority {
            host: AuthorityHost::Localhost,
            port: PORT,
        };
        let v6 = ip_authority(IpAddr::V6(Ipv6Addr::LOCALHOST));

        // when
        let decisions = [
            OriginPolicy::check(Some("http://127.0.0.1:8080"), Some("same-origin"), &ip),
            OriginPolicy::check(None, Some("same-origin"), &ip),
            OriginPolicy::check(None, Some("none"), &ip),
            OriginPolicy::check(None, Some("future-value"), &ip),
            OriginPolicy::check(Some("http://localhost:8080"), None, &localhost),
            OriginPolicy::check(Some("HTTP://LOCALHOST:8080"), None, &localhost),
            OriginPolicy::check(Some("http://[::1]:8080"), None, &v6),
        ];

        // then
        assert_eq!(decisions, [OriginDecision::Allow; 7]);
    }

    #[test]
    fn should_reject_when_origin_conflicts_with_host() {
        // given
        let host = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let localhost = Authority {
            host: AuthorityHost::Localhost,
            port: PORT,
        };

        // when
        let decisions = [
            OriginPolicy::check(Some("http://evil.example"), None, &host),
            OriginPolicy::check(Some("http://evil.example:8080"), Some("same-site"), &host),
            OriginPolicy::check(Some("http://127.0.0.1:3000"), Some("same-site"), &host),
            OriginPolicy::check(Some("http://127.0.0.1"), None, &host),
            OriginPolicy::check(Some("http://localhost:8080"), None, &host),
            OriginPolicy::check(Some("http://127.0.0.1:8080"), None, &localhost),
            OriginPolicy::check(Some("http://[::1]:8080"), None, &host),
        ];

        // then
        assert_eq!(
            decisions,
            [OriginDecision::Reject(OriginReject::AuthorityMismatch); 7]
        );
    }

    #[test]
    fn should_reject_when_origin_is_null() {
        // given
        let host = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));

        // when
        let decision = OriginPolicy::check(Some("null"), Some("same-origin"), &host);

        // then
        assert_eq!(decision, OriginDecision::Reject(OriginReject::Opaque));
    }

    #[test]
    fn should_reject_when_origin_scheme_is_not_http() {
        // given
        let host = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));

        // when
        let decisions = [
            OriginPolicy::check(Some("https://127.0.0.1:8080"), None, &host),
            OriginPolicy::check(Some("file://127.0.0.1:8080"), None, &host),
        ];

        // then
        assert_eq!(
            decisions,
            [OriginDecision::Reject(OriginReject::SchemeNotHttp); 2]
        );
    }

    #[test]
    fn should_reject_when_origin_is_malformed() {
        // given
        let host = ip_authority(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let origins = [
            "127.0.0.1:8080",
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/path",
            "http://127.0.0.1:8080?q",
            "http://127.0.0.1:8080#f",
            "http://user@127.0.0.1:8080",
            "http://[::1%lo]:8080",
            "http://",
            "http://127.0.0.1:8080 http://evil.example",
            "",
        ];

        // when
        let decisions: Vec<_> = origins
            .iter()
            .map(|origin| OriginPolicy::check(Some(origin), None, &host))
            .collect();

        // then
        assert_eq!(
            decisions,
            vec![OriginDecision::Reject(OriginReject::Malformed); origins.len()]
        );
    }

    #[test]
    fn should_derive_listener_kind_when_constructing_bound_addr() {
        // given
        let mapped_loopback: IpAddr = "::ffff:127.0.0.1".parse().expect("valid literal");
        let addrs = [
            BoundAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 9)), PORT),
            BoundAddr::new(mapped_loopback, PORT),
            BoundAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), PORT),
            BoundAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), PORT),
        ];

        // when
        let kinds: Vec<_> = addrs
            .iter()
            .map(|addr| (addr.is_loopback(), addr.is_wildcard(), addr.port()))
            .collect();

        // then
        assert_eq!(
            kinds,
            [
                (true, false, PORT),
                (true, false, PORT),
                (false, true, PORT),
                (false, false, PORT),
            ]
        );
    }

    #[test]
    fn should_use_the_default_port_when_bracketed_ipv6_host_has_no_port() {
        // given
        let raw = "[::1]";

        // when
        let (_, port) = parse_authority(raw).expect("a bracketed address alone is valid");

        // then
        assert_eq!(port, DEFAULT_HTTP_PORT);
    }

    #[test]
    fn should_accept_port_zero_and_reject_leading_zeros_when_port_is_parsed() {
        // given
        let zero = "0";
        let padded = "00";

        // when
        let results = (parse_port(zero), parse_port(padded));

        // then
        assert_eq!(results, (Ok(0), Err(HostReject::Malformed)));
    }
}
