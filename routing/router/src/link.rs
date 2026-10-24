//! Turning a link the platform hands the app into a path a [`Route`] can parse.
//!
//! [`Route`]: crate::Route
/// The route path a link addresses: the part of the URL a route's
/// [`parse`](crate::Route::parse) is written against, including its query.
///
/// For a web link (`https://example.com/topic/42?ref=a`) that is the URL's path
/// and query. A custom-scheme link has no site to name, so its host is the first
/// segment of the path: `myapp://topic/42` addresses `/topic/42`, exactly as
/// `myapp:///topic/42` does. A fragment is dropped, and a bare path
/// (`/topic/42`) is taken as it is. Anything else, including an empty string or
/// a relative path with no scheme, addresses nothing.
pub(crate) fn route_path(url: &str) -> Option<String> {
    if url.starts_with('/') && !url.starts_with("//") {
        return Some(url.split('#').next().unwrap_or(url).to_owned());
    }
    let parsed = url::Url::parse(url).ok()?;
    let mut path = String::new();
    // A web link's host is the site, which is not part of any route; a custom
    // scheme's host is where its path begins.
    let is_web = matches!(parsed.scheme(), "http" | "https");
    if !is_web && let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) {
        path.push('/');
        path.push_str(host);
    }
    if parsed.cannot_be_a_base() {
        // `myapp:topic/42` has no authority, and its path carries no leading slash.
        path.push('/');
    }
    path.push_str(parsed.path());
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_scheme_makes_its_host_the_first_segment() {
        assert_eq!(route_path("myapp://topic/42").as_deref(), Some("/topic/42"));
        assert_eq!(route_path("myapp://topic").as_deref(), Some("/topic"));
        assert_eq!(route_path("myapp:///topic/42").as_deref(), Some("/topic/42"), "an empty host is the same address");
    }

    #[test]
    fn a_web_link_is_its_path_and_the_host_names_the_site_not_a_segment() {
        assert_eq!(route_path("https://example.com/topic/42").as_deref(), Some("/topic/42"));
        assert_eq!(route_path("http://example.com").as_deref(), Some("/"));
        assert_eq!(route_path("https://example.com/").as_deref(), Some("/"));
    }

    #[test]
    fn the_query_is_kept_and_the_fragment_is_dropped() {
        assert_eq!(route_path("myapp://topic/42?ref=a&b=c#top").as_deref(), Some("/topic/42?ref=a&b=c"));
        assert_eq!(route_path("https://example.com/search?q=x#r").as_deref(), Some("/search?q=x"));
        assert_eq!(route_path("/topic/42?x=1#frag").as_deref(), Some("/topic/42?x=1"));
    }

    #[test]
    fn a_scheme_with_no_slashes_is_still_a_path() {
        assert_eq!(route_path("myapp:topic/42").as_deref(), Some("/topic/42"));
    }

    #[test]
    fn a_bare_absolute_path_is_taken_as_it_is() {
        assert_eq!(route_path("/topic/42").as_deref(), Some("/topic/42"));
        assert_eq!(route_path("/").as_deref(), Some("/"));
    }

    #[test]
    fn percent_encoding_is_left_for_the_route_to_decode() {
        assert_eq!(route_path("myapp://tag/rust%20lang").as_deref(), Some("/tag/rust%20lang"));
    }

    #[test]
    fn what_is_not_a_link_addresses_nothing() {
        for url in ["", "   ", "topic/42", "//host/path", "not a url", "://nope"] {
            assert_eq!(route_path(url), None, "{url:?}");
        }
    }
}
