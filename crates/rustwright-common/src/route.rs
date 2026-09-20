//! Request interception rules.
//!
//! Routes are declarative data (not callbacks), which keeps the interceptor
//! simple, `Send + Sync`, and easy to reason about. Matching never encodes
//! site-specific behaviour.

/// What to do with a matched request.
#[derive(Debug, Clone)]
pub enum RouteAction {
    /// Let the request continue unchanged.
    Continue,
    /// Abort the request as if the connection failed.
    Abort,
    /// Answer the request locally with a synthetic response.
    Fulfill {
        /// HTTP status code.
        status: i64,
        /// `Content-Type` header value.
        content_type: String,
        /// Response body.
        body: Vec<u8>,
    },
    /// Continue the request, overriding (or adding) these request headers.
    ///
    /// Provided headers are merged onto the original request headers.
    SetRequestHeaders(Vec<(String, String)>),
    /// Continue the response, overriding (or adding) these response headers.
    ///
    /// Provided headers are merged onto the original response headers.
    SetResponseHeaders(Vec<(String, String)>),
}

/// A URL pattern and the action to apply.
#[derive(Debug, Clone)]
pub struct Route {
    /// URL pattern. `*`/`?` are wildcards; without wildcards this is a
    /// case-sensitive substring match.
    pub pattern: String,
    /// The action to apply when the pattern matches.
    pub action: RouteAction,
}

impl Route {
    /// Create a route.
    pub fn new(pattern: impl Into<String>, action: RouteAction) -> Self {
        Self {
            pattern: pattern.into(),
            action,
        }
    }

    /// Whether this route matches `url`.
    pub fn matches(&self, url: &str) -> bool {
        if self.pattern.contains('*') || self.pattern.contains('?') {
            glob_match(&self.pattern, url)
        } else {
            url.contains(&self.pattern)
        }
    }
}

/// A small glob matcher supporting `*` (any sequence) and `?` (one character).
///
/// Matching is anchored at both ends.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut match_at = 0usize;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            match_at = t;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            match_at += 1;
            t = match_at;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_matches_when_no_wildcards() {
        let route = Route::new("api.example.com", RouteAction::Abort);
        assert!(route.matches("https://api.example.com/v1"));
        assert!(!route.matches("https://other.example.com/v1"));
    }

    #[test]
    fn glob_matches_wildcards() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("**/api/*", "https://x.test/api/users"));
        assert!(glob_match("https://x.test/*", "https://x.test/a/b"));
        assert!(!glob_match("https://x.test/*", "https://y.test/a"));
        assert!(glob_match("img?.png", "img1.png"));
        assert!(!glob_match("img?.png", "img10.png"));
    }

    #[test]
    fn route_uses_glob_when_wildcards_present() {
        let route = Route::new("**/ads/**", RouteAction::Abort);
        assert!(route.matches("https://site.test/ads/banner.js"));
        assert!(!route.matches("https://site.test/content.js"));
    }
}
