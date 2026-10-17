// Website matching for "turn off on these sites".

/// "https://www.YouTube.com/watch" -> "youtube.com/watch"
pub fn normalize(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(&s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.trim_end_matches('/').to_string()
}

/// A site matches its own host, its subdomains, and (if it has a path) anything under that path.
pub fn matches(url: &str, sites: &[String]) -> bool {
    let url = normalize(url);
    let host = url.split(['/', '?', '#']).next().unwrap_or("");
    sites.iter().any(|site| {
        if site.contains('/') {
            url.starts_with(site.as_str())
        } else {
            host == site || host.ends_with(&format!(".{site}"))
        }
    })
}

/// Process names (lowercase, no extension) of common browsers.
pub const BROWSERS: [&str; 11] = [
    "chrome", "msedge", "brave", "firefox", "opera", "vivaldi", "arc", "zen", "librewolf", "waterfox", "floorp",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_matching() {
        let sites = vec![normalize("YouTube.com"), normalize("https://github.com/settings/")];
        assert!(matches("youtube.com/watch?v=1", &sites));
        assert!(matches("https://www.youtube.com", &sites));
        assert!(matches("m.youtube.com/feed", &sites));
        assert!(!matches("notyoutube.com", &sites));
        assert!(!matches("youtube.com.evil.net/x", &sites));
        assert!(matches("github.com/settings/profile", &sites));
        assert!(!matches("github.com/BeForce1", &sites));
    }
}
