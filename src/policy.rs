use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

pub const HOME: &str = "fastium://newtab";
pub const BLOCKED_HOSTS: &str = include_str!("../assets/blocklist.txt");

static BLOCKED_RULES: OnceLock<HashSet<String>> = OnceLock::new();

fn blocked_rules() -> &'static HashSet<String> {
    BLOCKED_RULES.get_or_init(|| {
        BLOCKED_HOSTS
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .map(|s| s.trim_end_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// Fast host extraction from a URL string without full URL parsing.
/// Extracts the host portion (handling scheme, userinfo, port, IPv6) and trims trailing dot.
pub fn extract_host(url: &str) -> Option<&str> {
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    if scheme.is_empty()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
    {
        return None;
    }

    let rest = &url[scheme_end + 3..];
    let auth_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    if authority.is_empty() {
        return None;
    }

    let without_user = match authority.rfind('@') {
        Some(pos) => &authority[pos + 1..],
        None => authority,
    };
    if without_user.is_empty() {
        return None;
    }

    let host = if without_user.starts_with('[') {
        let close = without_user.find(']')?;
        &without_user[..=close]
    } else {
        match without_user.find(':') {
            Some(pos) => &without_user[..pos],
            None => without_user,
        }
    };

    let host = host.trim_end_matches('.');
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Only web URLs enter content views. Credentials and privileged/local schemes are rejected.
pub fn web_url(input: &str) -> Option<String> {
    let url = Url::parse(input).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
        .then(|| url.to_string())
}

pub fn resolve(input: &str) -> Result<String, &'static str> {
    let input = input.trim();
    if input.is_empty() || input == HOME { return Ok(HOME.into()); }
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("Adres jest zbyt długi lub zawiera znaki sterujące.");
    }
    // Explicit schemes never fall through to an executable/local URL.
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return web_url(input).ok_or("Niepoprawny adres HTTP/HTTPS.");
    }
    let host_part = input.split('/').next().unwrap_or(input);
    let candidate = if host_part.starts_with("localhost")
        || host_part.starts_with("127.") || host_part.starts_with("[::1]") {
        format!("http://{input}")
    } else { format!("https://{input}") };
    if !input.chars().any(char::is_whitespace)
        && !input.contains('@')
        && (host_part.contains('.') || host_part.starts_with('[')
            || host_part == "localhost" || host_part.starts_with("localhost:"))
    {
        if let Some(url) = web_url(&candidate) { return Ok(url); }
    }
    if let Some((scheme, _)) = input.split_once(':') {
        if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return Err("Obsługiwane są adresy HTTP i HTTPS.");
        }
    }
    let mut url = Url::parse("https://www.google.com/search").expect("constant URL");
    url.query_pairs_mut().append_pair("q", input);
    Ok(url.into())
}

pub fn blocked(url: &str) -> bool {
    let Some(raw_host) = extract_host(url) else {
        return false;
    };

    let host: Cow<str> = if raw_host.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(raw_host.to_ascii_lowercase())
    } else {
        Cow::Borrowed(raw_host)
    };

    let rules = blocked_rules();
    if rules.contains(host.as_ref()) {
        return true;
    }

    let mut remainder = host.as_ref();
    while let Some(dot_idx) = remainder.find('.') {
        remainder = &remainder[dot_idx + 1..];
        if !remainder.is_empty() && rules.contains(remainder) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omnibox_addresses_and_search() {
        for (input, expected) in [
            ("", HOME), ("example.org", "https://example.org/"),
            ("http://example.org/a", "http://example.org/a"),
            ("localhost:8000", "http://localhost:8000/"),
            ("127.0.0.1:8080/a", "http://127.0.0.1:8080/a"),
            ("[::1]:8000", "http://[::1]:8000/"),
        ] { assert_eq!(resolve(input).unwrap(), expected); }
        let url = Url::parse(&resolve("żółw & Rust #1").unwrap()).unwrap();
        assert_eq!(url.query_pairs().next().unwrap().1, "żółw & Rust #1");
    }
    #[test]
    fn privileged_navigation_and_credentials_are_rejected() {
        for input in ["javascript:alert(1)", "file:///C:/secret", "data:text/html,x", "https://user:pass@example.org"] {
            assert!(resolve(input).is_err(), "{input}");
            assert!(web_url(input).is_none());
        }
        assert!(resolve("https://example.org/\nfoo").is_err());
    }
    #[test]
    fn filters_match_host_boundaries_only() {
        assert!(blocked("https://ads.doubleclick.net/ad.js"));
        assert!(blocked("https://DOUBLECLICK.NET./ad.js"));
        assert!(!blocked("https://notdoubleclick.net/"));
        assert!(!blocked("https://doubleclick.net.example.org/"));
        assert!(!blocked("https://example.org/?ad=doubleclick.net"));
    }
    #[test]
    fn blocked_edge_cases() {
        // Uppercase, mixed case, trailing dot
        assert!(blocked("https://DOUBLECLICK.NET/ad.js"));
        assert!(blocked("https://DoubleClick.Net./ad.js"));
        assert!(blocked("https://ADS.DOUBLECLICK.NET./ad.js"));
        assert!(blocked("http://Ads.DoubleClick.Net:8080/ad.js"));

        // Subdomain depth
        assert!(blocked("https://a.b.c.doubleclick.net/test"));
        assert!(blocked("https://deep.sub.googleadservices.com/page"));

        // Exact rule vs non-matching suffixes
        assert!(!blocked("https://notdoubleclick.net/"));
        assert!(!blocked("https://mydoubleclick.net/"));
        assert!(!blocked("https://doubleclick.net.example.org/"));

        // Query string, path, or fragment containing blocked domain must NOT trigger false positive
        assert!(!blocked("https://example.org/?redirect=doubleclick.net"));
        assert!(!blocked("https://example.org/path/doubleclick.net"));
        assert!(!blocked("https://example.org#doubleclick.net"));
        assert!(!blocked("https://example.com/search?q=https://doubleclick.net/"));

        // Ports and userinfo
        assert!(blocked("https://user:pass@doubleclick.net:8443/ad"));
        assert!(blocked("https://user@ads.doubleclick.net:443/"));

        // No host or non-web schemes
        assert!(!blocked(""));
        assert!(!blocked("https:///path"));
        assert!(!blocked("javascript:alert('doubleclick.net')"));
        assert!(!blocked("data:text/html,doubleclick.net"));
        assert!(!blocked("about:blank"));
        assert!(!blocked("fastium://newtab"));
        assert!(!blocked("http://[::1]:8080/doubleclick.net"));
    }
}

