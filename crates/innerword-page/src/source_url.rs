//! The normal form of the address a page came from (docs/pages-design.md, section 1).

use url::Url;

use crate::error::PageError;

const NOT_HTTP: &str = "the source address must be an http or https address";

/// The normal form of a source address: scheme and host lowercased, the default port,
/// the `#fragment` and a trailing `/` of the path dropped, the query kept. Err with the
/// sentence for one that is not http(s).
pub fn normal_source_url(url: &str) -> Result<String, PageError> {
    let parsed = Url::parse(url.trim()).map_err(|_| PageError::input(NOT_HTTP))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(PageError::input(NOT_HTTP));
    }
    let Some(host) = parsed.host_str() else {
        return Err(PageError::input(NOT_HTTP));
    };
    let mut out = format!("{scheme}://");
    if !parsed.username().is_empty() {
        out.push_str(parsed.username());
        if let Some(password) = parsed.password() {
            out.push(':');
            out.push_str(password);
        }
        out.push('@');
    }
    out.push_str(host);
    // `port()` is None for the default port of the scheme.
    if let Some(port) = parsed.port() {
        out.push(':');
        out.push_str(&port.to_string());
    }
    out.push_str(parsed.path().trim_end_matches('/'));
    if let Some(query) = parsed.query().filter(|query| !query.is_empty()) {
        out.push('?');
        out.push_str(query);
    }
    Ok(out)
}
