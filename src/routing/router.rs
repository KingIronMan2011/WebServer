//! Selects a handler from the request host and path.

use crate::config::{Config, RouteConfig, SiteConfig, normalise_host};

pub struct MatchedRoute<'a> {
    pub site: &'a SiteConfig,
    pub route: &'a RouteConfig,
}

pub fn find<'a>(config: &'a Config, host: &str, path: &str) -> Option<MatchedRoute<'a>> {
    let host = normalise_host(host);
    let site = config
        .sites
        .iter()
        .find(|site| normalise_host(&site.host) == host)?;
    site.routes
        .iter()
        .filter(|route| path_matches(&route.path_prefix, path))
        .max_by_key(|route| route.path_prefix.len())
        .map(|route| MatchedRoute { site, route })
}

fn path_matches(prefix: &str, path: &str) -> bool {
    prefix == "/"
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::path_matches;
    #[test]
    fn only_matches_path_segment_boundaries() {
        assert!(path_matches("/api", "/api/v1"));
        assert!(!path_matches("/api", "/apix"));
    }
}
