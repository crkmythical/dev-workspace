//! Judging a request from its headers (pure; the HTTP layer reads the headers and calls these).
//!
//! Two questions the gate asks beyond "is there a valid session":
//!
//! * Is this a WebSocket handshake from a page on ANOTHER origin? The session cookie is
//!   `SameSite=Lax`, which keeps it off requests from other SITES, but a site is a registrable
//!   domain: a page on a sibling subdomain (another workspace on the same domain, any app that
//!   shares it) is same-site, so its handshake would carry the cookie. Browsers always send an
//!   `Origin` on a WebSocket handshake, so comparing it with the host being asked closes that.
//! * Did the client reach us over HTTPS? Local access is plain HTTP on loopback; the remote leg is
//!   an HTTPS edge (the tunnel) that says so: Cloudflare sends `CF-Visitor: {"scheme":"https"}`,
//!   which Caddy passes on untouched. (`X-Forwarded-Proto` is read too, but Caddy replaces it for
//!   the services behind it unless the peer is a configured trusted proxy, so it is no more than a
//!   fallback.) Only then is `Secure` set on the cookie, since a `Secure` cookie would lock a
//!   plain-HTTP localhost browser out (Safari).

/// `Upgrade: websocket` (the value is a case-insensitive token list).
pub fn is_websocket_upgrade(upgrade: Option<&str>) -> bool {
    upgrade.is_some_and(|v| {
        v.split(',')
            .any(|t| t.trim().eq_ignore_ascii_case("websocket"))
    })
}

/// `host[:port]` with a port that is the scheme's default removed, lower-cased.
fn normalize_authority(authority: &str, scheme: &str) -> String {
    let a = authority.trim().to_ascii_lowercase();
    let default = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => return a,
    };
    a.strip_suffix(default).map(str::to_string).unwrap_or(a)
}

/// Whether an `Origin` header names `host` (the `Host` the request was addressed to). `null` (a
/// sandboxed frame, a file), a scheme other than http(s), credentials in the origin, a path and
/// anything unparsable are all "no".
pub fn origin_matches_host(origin: &str, host: &str) -> bool {
    let origin = origin.trim();
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return false;
    }
    // An origin is scheme://host[:port] and nothing after it.
    if rest.is_empty() || rest.contains(['/', '@', '?', '#', ' ']) {
        return false;
    }
    normalize_authority(rest, &scheme) == normalize_authority(host, &scheme)
}

/// A WebSocket handshake whose `Origin` is not the host it is addressed to. A handshake without an
/// `Origin` is not a browser's (browsers always send it), so no ambient cookie is at stake and it
/// passes. `forwarded_host` is the X-Forwarded-Host the proxy sets; either name is accepted.
pub fn is_cross_origin_websocket(
    upgrade: Option<&str>,
    origin: Option<&str>,
    host: Option<&str>,
    forwarded_host: Option<&str>,
) -> bool {
    if !is_websocket_upgrade(upgrade) {
        return false;
    }
    let Some(origin) = origin else {
        return false;
    };
    let named = [host, forwarded_host].into_iter().flatten();
    !named.into_iter().any(|h| origin_matches_host(origin, h))
}

/// Whether the client's connection to the edge was HTTPS: `X-Forwarded-Proto` (the first hop's
/// value, which is the client-facing one) or Cloudflare's `CF-Visitor: {"scheme":"https"}`.
pub fn arrived_over_https(forwarded_proto: Option<&str>, cf_visitor: Option<&str>) -> bool {
    let proto = forwarded_proto
        .and_then(|v| v.split(',').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"));
    let visitor = cf_visitor.is_some_and(|v| {
        let compact: String = v.chars().filter(|c| !c.is_whitespace()).collect();
        compact
            .to_ascii_lowercase()
            .contains("\"scheme\":\"https\"")
    });
    proto || visitor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_upgrades_are_recognised_whatever_the_case() {
        assert!(is_websocket_upgrade(Some("websocket")));
        assert!(is_websocket_upgrade(Some("WebSocket")));
        assert!(is_websocket_upgrade(Some("h2c, websocket")));
        assert!(!is_websocket_upgrade(Some("h2c")));
        assert!(!is_websocket_upgrade(Some("websockets")));
        assert!(!is_websocket_upgrade(None));
    }

    #[test]
    fn an_origin_names_its_own_host_and_nothing_else() {
        for (origin, host) in [
            ("https://ws.example.com", "ws.example.com"),
            ("HTTPS://WS.Example.com", "ws.example.com"),
            ("http://localhost:18080", "localhost:18080"),
            ("http://127.0.0.1:18090", "127.0.0.1:18090"),
            // The default port may be written or left out on either side.
            ("https://ws.example.com:443", "ws.example.com"),
            ("https://ws.example.com", "ws.example.com:443"),
            ("http://ws.example.com:80", "ws.example.com"),
        ] {
            assert!(origin_matches_host(origin, host), "{origin} vs {host}");
        }
        for (origin, host) in [
            ("https://evil.example", "ws.example.com"),
            // A sibling subdomain is the same SITE but not the same origin.
            ("https://other.example.com", "ws.example.com"),
            ("https://ws.example.com.evil.example", "ws.example.com"),
            ("https://evil.example/ws.example.com", "ws.example.com"),
            ("https://user@ws.example.com", "ws.example.com"),
            ("http://localhost:18080", "localhost:9999"),
            ("http://localhost", "localhost:18080"),
            ("null", "ws.example.com"),
            ("", "ws.example.com"),
            ("ws.example.com", "ws.example.com"),
            ("ftp://ws.example.com", "ws.example.com"),
            ("https://", "ws.example.com"),
        ] {
            assert!(!origin_matches_host(origin, host), "{origin} vs {host}");
        }
    }

    #[test]
    fn only_a_cross_origin_websocket_handshake_is_flagged() {
        let ws = Some("websocket");
        // Same origin, and no Origin at all (not a browser): fine.
        assert!(!is_cross_origin_websocket(
            ws,
            Some("https://a.example"),
            Some("a.example"),
            None
        ));
        assert!(!is_cross_origin_websocket(
            ws,
            None,
            Some("a.example"),
            None
        ));
        // Another origin, even a sibling subdomain, or `null`.
        assert!(is_cross_origin_websocket(
            ws,
            Some("https://b.example"),
            Some("a.example"),
            None
        ));
        assert!(is_cross_origin_websocket(
            ws,
            Some("null"),
            Some("a.example"),
            None
        ));
        // The proxy's X-Forwarded-Host may be the name the browser used.
        assert!(!is_cross_origin_websocket(
            ws,
            Some("https://a.example"),
            Some("127.0.0.1:8080"),
            Some("a.example")
        ));
        // With no host at all, a browser handshake cannot be shown to be same-origin.
        assert!(is_cross_origin_websocket(
            ws,
            Some("https://a.example"),
            None,
            None
        ));
        // Plain requests are not this check's business.
        assert!(!is_cross_origin_websocket(
            None,
            Some("https://b.example"),
            Some("a.example"),
            None
        ));
    }

    #[test]
    fn https_is_read_from_the_edge_headers() {
        assert!(arrived_over_https(Some("https"), None));
        assert!(arrived_over_https(Some("HTTPS"), None));
        assert!(arrived_over_https(Some("https, http"), None));
        assert!(arrived_over_https(None, Some(r#"{"scheme":"https"}"#)));
        assert!(arrived_over_https(None, Some(r#"{ "scheme" : "https" }"#)));
        assert!(!arrived_over_https(Some("http"), None));
        assert!(!arrived_over_https(Some("http, https"), None));
        assert!(!arrived_over_https(None, Some(r#"{"scheme":"http"}"#)));
        assert!(!arrived_over_https(None, None));
    }
}
