//! Parsing of the errex (Sentry-protocol) DSN into an ingest endpoint.

/// A parsed DSN, reduced to the one URL the client actually posts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dsn {
    /// Fully-qualified envelope endpoint, auth query string included.
    pub ingest_url: String,
}

impl Dsn {
    /// Parse `<scheme>://<public_key>@<host>[:port]/<project>`.
    ///
    /// Returns `None` for anything malformed — a bad DSN disables reporting
    /// rather than failing the page. The DSN public key is not a secret (every
    /// browser SDK ships it), but it is still validated to a conservative
    /// character set so a mistyped meta tag cannot inject URL syntax.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let (scheme, rest) = raw.trim().split_once("://")?;
        if !matches!(scheme, "http" | "https") {
            return None;
        }
        let (credentials, rest) = rest.split_once('@')?;
        // Legacy DSNs carry `<public_key>:<secret_key>`; only the public half
        // is ever sent from a browser.
        let public_key = credentials.split(':').next()?;
        let (host, project) = rest.split_once('/')?;
        let project = project.trim_end_matches('/');

        if !is_token(public_key) || !is_token(project) || !is_host(host) {
            return None;
        }

        Some(Self {
            ingest_url: format!(
                "{scheme}://{host}/api/{project}/envelope/?sentry_key={public_key}&sentry_version=7&sentry_client={}%2F{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ),
        })
    }
}

/// Non-empty and free of URL-structural characters.
fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Same, plus the `:` of an explicit port.
fn is_host(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

#[cfg(test)]
mod tests {
    use super::Dsn;

    #[test]
    fn parses_the_errex_dsn_shape() {
        let dsn = Dsn::parse(
            "https://00000000000000000000000000000000@errex.example.internal/random.cash",
        )
        .expect("valid DSN");
        assert!(
            dsn.ingest_url
                .starts_with("https://errex.example.internal/api/random.cash/envelope/?"),
            "{}",
            dsn.ingest_url
        );
        assert!(
            dsn.ingest_url
                .contains("sentry_key=00000000000000000000000000000000"),
            "{}",
            dsn.ingest_url
        );
    }

    #[test]
    fn keeps_an_explicit_port_and_drops_the_secret_half() {
        let dsn = Dsn::parse("http://pub:secret@127.0.0.1:9000/2").expect("valid DSN");
        assert!(
            dsn.ingest_url
                .starts_with("http://127.0.0.1:9000/api/2/envelope/?")
        );
        assert!(!dsn.ingest_url.contains("secret"), "{}", dsn.ingest_url);
    }

    #[test]
    fn tolerates_surrounding_whitespace_and_a_trailing_slash() {
        let dsn = Dsn::parse("  https://key@errex.example/proj/  ").expect("valid DSN");
        assert!(
            dsn.ingest_url
                .starts_with("https://errex.example/api/proj/envelope/?")
        );
    }

    #[test]
    fn rejects_malformed_or_unsafe_dsns() {
        for raw in [
            "",
            "not-a-dsn",
            "https://errex.example/random.cash", // no public key
            "https://key@errex.example",         // no project
            "ftp://key@errex.example/proj",      // unsupported scheme
            "https://key@errex.example/proj?evil=1", // query injection
            "https://key@errex.example/proj/../../other", // path traversal
        ] {
            assert!(Dsn::parse(raw).is_none(), "should reject: {raw}");
        }
    }
}
