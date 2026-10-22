use url::Url;

pub const HOME: &str = "fastium://newtab";
pub const BLOCKED_HOSTS: &str = include_str!("../assets/blocklist.txt");

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
    let Ok(url) = Url::parse(url) else { return false };
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_end_matches('.');
    BLOCKED_HOSTS.lines().map(str::trim).filter(|s| !s.is_empty() && !s.starts_with('#'))
        .any(|rule| host == rule || host.strip_suffix(rule).is_some_and(|prefix| prefix.ends_with('.')))
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
}

