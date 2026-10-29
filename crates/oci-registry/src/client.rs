//! `Client`: `head_digest` and `resolve`, with the exact request set of the contract.
use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};

use crate::auth::Credentials;
use crate::error::RegistryError;
use crate::manifest::Platform;
use crate::reference::{Digest, ImageRef};

/// The requests one call made, success or failure.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RequestCounts {
    /// Every HEAD.
    pub head: u32,
    /// Every GET: token, manifest and blob.
    pub get: u32,
    /// The GETs on `/v2/*/manifests/*`, which registries count as pulls.
    pub manifest_get: u32,
}

/// Every call returns its result together with the requests it made, success or failure.
#[derive(Debug)]
pub struct Counted<T> {
    pub result: Result<T, RegistryError>,
    pub requests: RequestCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Set when the digest named an index or manifest list.
    pub index_digest: Option<Digest>,
    /// The platform manifest.
    pub manifest_digest: Digest,
    pub config_digest: Digest,
    /// `config.Labels`, empty when absent.
    pub labels: BTreeMap<String, String>,
    /// `config.User`.
    pub user: Option<String>,
    /// The config's top-level `created`, as given.
    pub created: Option<String>,
}

pub struct Client {}

impl Client {
    /// `https` only. `user_agent` is the caller's, e.g. `keel/0.2.0 (update-check)`.
    pub fn new(user_agent: &str) -> Result<Client, RegistryError> {
        let _ = user_agent;
        todo!()
    }

    /// Feature `test-util` only: every registry and realm request goes to `base` (plain `http`
    /// allowed), so tests point the client at the fake registry.
    #[cfg(feature = "test-util")]
    pub fn with_base_url(user_agent: &str, base: reqwest::Url) -> Client {
        let _ = (user_agent, base);
        todo!()
    }

    /// HEAD the manifest. Never falls back to GET.
    pub async fn head_digest(&self, r: &ImageRef, c: &Credentials) -> Counted<Digest> {
        let _ = (r, c);
        todo!()
    }

    /// `r` must be by digest (`NotByDigest` otherwise, with no request made).
    pub async fn resolve(&self, r: &ImageRef, p: &Platform, c: &Credentials) -> Counted<Resolved> {
        let _ = (r, p, c);
        todo!()
    }
}

/// `sha256:<64 lowercase hex>` of `body`.
pub(crate) fn sha256_digest(body: &[u8]) -> String {
    let mut out = String::with_capacity(71);
    out.push_str("sha256:");
    for b in Sha256::digest(body).iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}
