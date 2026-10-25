//! T093: response headers on everything Depths serves.
//!
//! - A full CSP: nothing loads unless named, scripts only from this origin, no `unsafe-eval`, no
//!   `unsafe-inline` scripts, no framing.
//! - HSTS (when the public URL is https), `nosniff`, `no-referrer`, a closed Permissions-Policy,
//!   COOP and CORP `same-origin`.
//! - `Cache-Control: no-store` on `/auth` and `/api`; `immutable` only on `/assets` (content-
//!   hashed by the build); everything else `no-cache`, so a new `index.html` is picked up.
//! - No `Server` header and no CORS headers, ever.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use http::{HeaderValue, header};

/// `style-src 'unsafe-inline'` because the web shell's toaster (sonner) injects its stylesheet as
/// a `<style>` element at runtime, and CodeMirror and xterm do the same. Styles only: scripts
/// stay `'self'`, with no `unsafe-inline` and no `unsafe-eval`.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' \
                       data:; font-src 'self'; connect-src 'self'; manifest-src 'self'; \
                       base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
pub const HSTS: &str = "max-age=63072000; includeSubDomains";
pub const PERMISSIONS_POLICY: &str = "accelerometer=(), autoplay=(), camera=(), \
                                      display-capture=(), geolocation=(), gyroscope=(), \
                                      magnetometer=(), microphone=(), midi=(), payment=(), \
                                      usb=(), interest-cohort=()";

/// What the path's caching policy is.
pub fn cache_control(path: &str) -> &'static str {
    if is_under(path, "/api") || is_under(path, "/auth") {
        "no-store"
    } else if is_under(path, "/assets") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn is_under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The layer, as `axum::middleware::from_fn_with_state(hsts, security_headers)`.
pub async fn security_headers(
    axum::extract::State(hsts): axum::extract::State<bool>,
    req: Request,
    next: Next,
) -> Response {
    let cache = cache_control(req.uri().path());
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    if hsts {
        h.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        "permissions-policy",
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.remove(header::SERVER);
    for cors in [
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        header::ACCESS_CONTROL_ALLOW_METHODS,
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
    ] {
        h.remove(cors);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caching_by_path() {
        assert_eq!(cache_control("/api/v1/me"), "no-store");
        assert_eq!(cache_control("/auth/login"), "no-store");
        assert_eq!(cache_control("/api"), "no-store");
        assert_eq!(
            cache_control("/assets/index-abc123.js"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control("/"), "no-cache");
        assert_eq!(cache_control("/index.html"), "no-cache");
        // Not a prefix match on the name.
        assert_eq!(cache_control("/assetsx/a.js"), "no-cache");
        assert_eq!(cache_control("/apix"), "no-cache");
    }

    #[test]
    fn the_csp_is_strict() {
        assert!(CSP.starts_with("default-src 'none'"));
        assert!(CSP.contains("script-src 'self';"));
        assert!(!CSP.contains("unsafe-eval"));
        // Inline is allowed for styles only.
        let script = CSP
            .split(';')
            .find(|d| d.trim().starts_with("script-src"))
            .unwrap();
        assert_eq!(script.trim(), "script-src 'self'");
        assert_eq!(CSP.matches("unsafe-inline").count(), 1);
        assert!(CSP.contains("frame-ancestors 'none'"));
    }
}
