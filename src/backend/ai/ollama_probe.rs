//! Ollama detection (038 FR-010..013): `/api/version` + `/api/tags` with a 3 s timeout,
//! a PATH scan for an installed-but-not-running hint, and no caching.

use std::time::Duration;

use serde::Deserialize;

use super::http::AiHttp;
use crate::protocol::v1::{InstalledHint, OllamaModel, OllamaState};

pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
struct Version {
    version: Option<String>,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    modified_at: String,
    #[serde(default)]
    details: Option<Details>,
}

#[derive(Deserialize)]
struct Details {
    family: Option<String>,
    parameter_size: Option<String>,
}

/// Is an `ollama` executable on PATH? A hint only; never used to decide reachability.
pub(crate) fn installed_hint() -> InstalledHint {
    let Some(path) = std::env::var_os("PATH") else {
        return InstalledHint::Unknown;
    };
    let names: &[&str] = if cfg!(windows) {
        &["ollama.exe", "ollama.cmd", "ollama"]
    } else {
        &["ollama"]
    };
    let found = std::env::split_paths(&path).any(|dir| names.iter().any(|n| dir.join(n).is_file()));
    if found {
        InstalledHint::Found
    } else {
        InstalledHint::NotFound
    }
}

pub(crate) async fn probe(http: &AiHttp, endpoint: &str) -> OllamaState {
    probe_with_hint(http, endpoint, installed_hint()).await
}

pub(crate) async fn probe_with_hint(
    http: &AiHttp,
    endpoint: &str,
    hint: InstalledHint,
) -> OllamaState {
    let base = endpoint.trim_end_matches('/');
    let checked_at = chrono::Utc::now().to_rfc3339();
    let mut state = OllamaState {
        endpoint: endpoint.to_owned(),
        reachable: false,
        version: None,
        installed_hint: hint,
        models: Vec::new(),
        checked_at,
        error: None,
    };

    let version = match http.get(&format!("{base}/api/version")) {
        Ok(req) => req.timeout(PROBE_TIMEOUT).send().await,
        Err(e) => {
            state.error = Some(e.to_string());
            return state;
        }
    };
    match version {
        Ok(resp) if resp.status().is_success() => {
            state.reachable = true;
            state.version = resp.json::<Version>().await.ok().and_then(|v| v.version);
        }
        Ok(resp) => {
            state.error = Some(format!("HTTP {} from {base}/api/version", resp.status()));
            return state;
        }
        Err(e) => {
            state.error = Some(if e.is_offline_blocked() {
                e.to_string()
            } else {
                format!("Could not reach Ollama at {endpoint}: {e}")
            });
            return state;
        }
    }

    let tags = match http.get(&format!("{base}/api/tags")) {
        Ok(req) => req.timeout(PROBE_TIMEOUT).send().await,
        Err(e) => {
            state.error = Some(e.to_string());
            return state;
        }
    };
    match tags {
        Ok(resp) if resp.status().is_success() => match resp.json::<Tags>().await {
            Ok(tags) => {
                state.models = tags
                    .models
                    .into_iter()
                    .map(|t| OllamaModel {
                        name: t.name,
                        size_bytes: t.size,
                        family: t.details.as_ref().and_then(|d| d.family.clone()),
                        parameter_size: t.details.and_then(|d| d.parameter_size),
                        modified_at: t.modified_at,
                    })
                    .collect();
            }
            Err(e) => state.error = Some(format!("Could not parse Ollama model list: {e}")),
        },
        Ok(resp) => state.error = Some(format!("HTTP {} from {base}/api/tags", resp.status())),
        Err(e) => state.error = Some(format!("Could not list Ollama models: {e}")),
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn stub(models: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"version":"0.5.1"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": models})),
            )
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn detected_with_models() {
        let server = stub(serde_json::json!([
            {"name":"llama3.2:latest","size":2019393189u64,"modified_at":"2025-01-01T00:00:00Z",
             "details":{"family":"llama","parameter_size":"3.2B"}},
            {"name":"qwen2.5:7b","size":4683087332u64,"modified_at":"2025-02-01T00:00:00Z"}
        ]))
        .await;
        let st = probe_with_hint(&AiHttp::new(), &server.uri(), InstalledHint::Found).await;
        assert!(st.reachable);
        assert_eq!(st.version.as_deref(), Some("0.5.1"));
        assert_eq!(st.models.len(), 2);
        assert_eq!(st.models[0].family.as_deref(), Some("llama"));
        assert_eq!(st.models[1].parameter_size, None);
        assert!(st.error.is_none());
    }

    #[tokio::test]
    async fn detected_with_no_models() {
        let server = stub(serde_json::json!([])).await;
        let st = probe_with_hint(&AiHttp::new(), &server.uri(), InstalledHint::Found).await;
        assert!(st.reachable);
        assert!(st.models.is_empty());
        assert!(st.error.is_none());
    }

    #[tokio::test]
    async fn not_running_reports_hint_and_error() {
        let st = probe_with_hint(&AiHttp::new(), "http://127.0.0.1:1", InstalledHint::Found).await;
        assert!(!st.reachable);
        assert_eq!(st.installed_hint, InstalledHint::Found);
        assert!(st.error.unwrap().contains("Could not reach Ollama"));
    }

    #[tokio::test]
    async fn timeout_is_bounded_and_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(6)))
            .mount(&server)
            .await;
        let started = std::time::Instant::now();
        let st = probe_with_hint(&AiHttp::new(), &server.uri(), InstalledHint::NotFound).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!st.reachable);
        assert!(st.error.is_some());
    }

    #[tokio::test]
    async fn no_cache_between_probes() {
        let server = stub(serde_json::json!([])).await;
        let http = AiHttp::new();
        let a = probe_with_hint(&http, &server.uri(), InstalledHint::Unknown).await;
        server.reset().await;
        let b = probe_with_hint(&http, &server.uri(), InstalledHint::Unknown).await;
        assert!(a.reachable);
        assert!(!b.reachable);
    }
}
