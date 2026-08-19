//! Which response headers may be replayed from cache.

/// Hop-by-hop and server-generated headers that must not be replayed from cache.
/// On replay we reconstruct these from the live response context.
pub(super) const NON_REPLAYABLE_HEADERS: &[&str] = &[
    "date",
    "server",
    "connection",
    "transfer-encoding",
    "content-length",
    "keep-alive",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "idempotency-replayed",
];

pub(super) fn is_replayable_header(name: &str) -> bool {
    !NON_REPLAYABLE_HEADERS
        .iter()
        .any(|d| name.eq_ignore_ascii_case(d))
}
/// Filter response headers to only the ones safe to replay, discarding
/// non-UTF8 values.
pub(super) fn collect_cacheable_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, val)| {
            let name_str = name.as_str();
            if !is_replayable_header(name_str) {
                return None;
            }
            val.to_str()
                .ok()
                .map(|v| (name_str.to_string(), v.to_string()))
        })
        .collect()
}
