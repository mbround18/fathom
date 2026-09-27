//! Building `reqwest` clients without panicking.
//!
//! reqwest's TLS setup reads the system CA store. On a machine without one (a minimal container,
//! or a distro where `ca-certificates` was removed) `ClientBuilder::build()` fails with "No CA
//! certificates were loaded from the system", and `reqwest::Client::new()` panics on the same
//! error. That panic used to take the whole backend down at startup, before any window state
//! could explain it. The helpers here fall back to a client with an empty trust store instead:
//! plain-HTTP requests still work, HTTPS requests fail per request with a certificate error, and
//! the app stays up.

use std::sync::OnceLock;

/// Build `make()`; if TLS setup fails, rebuild it with an empty root store and log why.
pub(crate) fn build_or_fallback(make: impl Fn() -> reqwest::ClientBuilder) -> reqwest::Client {
    match make().build() {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(
                "HTTP client setup failed ({err}); HTTPS requests will fail until the system CA \
                 certificates are installed (the `ca-certificates` package)"
            );
            make()
                .tls_certs_only(Vec::new())
                .build()
                .or_else(|_| {
                    reqwest::Client::builder()
                        .tls_certs_only(Vec::new())
                        .build()
                })
                .expect("a reqwest client with an empty trust store always builds")
        }
    }
}

/// A shared default client (the equivalent of `reqwest::Client::new()`, minus the panic).
pub(crate) fn shared() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| build_or_fallback(reqwest::Client::builder))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_builder_falls_back_instead_of_panicking() {
        // An empty TLS version range is rejected by `build()`; the fallback must still return a
        // client rather than panic.
        let client = build_or_fallback(|| {
            reqwest::Client::builder()
                .tls_version_min(reqwest::tls::Version::TLS_1_3)
                .tls_version_max(reqwest::tls::Version::TLS_1_2)
        });
        drop(client);
    }

    #[test]
    fn shared_client_builds() {
        drop(shared());
    }
}
