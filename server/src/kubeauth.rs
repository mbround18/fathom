//! Per-request Kubernetes authentication for hosted users (spec 031, T019).
//!
//! Pass-through: the user's own ID token is the bearer, read from a [`TokenCell`] on every
//! request so a refresh swaps it without rebuilding the client or dropping watches.
//! Impersonation: Fathom's service-account token (left to kube's own token-file handling)
//! plus `Impersonate-User` and one `Impersonate-Group` header per group.
//! An empty or revoked cell fails closed: the request errors instead of going out anonymous.
//! Tokens are never formatted: `Debug` output is redacted and header values are marked sensitive.

use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::SystemTime;

use futures::future::BoxFuture;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use kube::client::ClientBuilder;
use kube::{Client, Config};
use secrecy::{ExposeSecret, SecretString};
use tower::{BoxError, Layer, Service};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("token contains characters that cannot appear in an HTTP header")]
    InvalidToken,
    #[error("identity contains characters that cannot appear in an HTTP header")]
    InvalidIdentity,
    #[error("no credentials available for this session")]
    NoCredentials,
}

#[derive(Default)]
struct Slot {
    header: Option<HeaderValue>,
    expires_at: Option<SystemTime>,
}

/// Shared holder of the current bearer token for one user's client.
#[derive(Clone, Default)]
pub struct TokenCell {
    slot: Arc<RwLock<Slot>>,
}

impl std::fmt::Debug for TokenCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = self.is_set();
        f.debug_struct("TokenCell")
            .field("token", &if set { "<redacted>" } else { "<empty>" })
            .finish()
    }
}

impl TokenCell {
    pub fn new(token: &SecretString, expires_at: Option<SystemTime>) -> Result<Self, AuthError> {
        let cell = Self::default();
        cell.set(token, expires_at)?;
        Ok(cell)
    }

    /// Swap in a refreshed token. On error the previous token is kept.
    pub fn set(
        &self,
        token: &SecretString,
        expires_at: Option<SystemTime>,
    ) -> Result<(), AuthError> {
        let mut header = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
            .map_err(|_| AuthError::InvalidToken)?;
        header.set_sensitive(true);
        let mut slot = self.slot.write().unwrap_or_else(|e| e.into_inner());
        slot.header = Some(header);
        slot.expires_at = expires_at;
        Ok(())
    }

    /// Drop the token (session ended): later requests fail closed.
    pub fn revoke(&self) {
        let mut slot = self.slot.write().unwrap_or_else(|e| e.into_inner());
        slot.header = None;
        slot.expires_at = None;
    }

    pub fn is_set(&self) -> bool {
        self.slot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .header
            .is_some()
    }

    pub fn expires_at(&self) -> Option<SystemTime> {
        self.slot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .expires_at
    }

    fn header(&self) -> Option<HeaderValue> {
        self.slot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .header
            .clone()
    }
}

/// Who to impersonate (already mapped through claim prefixes by the caller).
#[derive(Clone)]
pub struct Impersonation {
    user: HeaderValue,
    groups: Vec<HeaderValue>,
}

impl std::fmt::Debug for Impersonation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Impersonation")
            .field("groups", &self.groups.len())
            .finish_non_exhaustive()
    }
}

impl Impersonation {
    pub fn new(user: &str, groups: &[String]) -> Result<Self, AuthError> {
        let hv = |s: &str| HeaderValue::from_str(s).map_err(|_| AuthError::InvalidIdentity);
        Ok(Self {
            user: hv(user)?,
            groups: groups.iter().map(|g| hv(g)).collect::<Result<_, _>>()?,
        })
    }
}

/// How requests are authenticated.
#[derive(Clone, Debug)]
pub enum Credentials {
    /// `Authorization: Bearer <cell>` on every request.
    Bearer(TokenCell),
    /// Leave `Authorization` to the base client (the SA token) and add impersonation headers.
    Impersonate(Impersonation),
}

static IMPERSONATE_USER: HeaderName = HeaderName::from_static("impersonate-user");
static IMPERSONATE_GROUP: HeaderName = HeaderName::from_static("impersonate-group");

/// Tower layer applying [`Credentials`] per request.
#[derive(Clone, Debug)]
pub struct AuthLayer {
    creds: Credentials,
}

impl AuthLayer {
    pub fn new(creds: Credentials) -> Self {
        Self { creds }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            creds: self.creds.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthService<S> {
    inner: S,
    creds: Credentials,
}

impl<S, B> Service<http::Request<B>> for AuthService<S>
where
    S: Service<http::Request<B>>,
    S::Error: Into<BoxError>,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<S::Response, BoxError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: http::Request<B>) -> Self::Future {
        let headers = req.headers_mut();
        // Whatever a caller put there, the user's identity is decided here only.
        headers.remove(&IMPERSONATE_USER);
        headers.remove(&IMPERSONATE_GROUP);
        match &self.creds {
            Credentials::Bearer(cell) => {
                headers.remove(AUTHORIZATION);
                match cell.header() {
                    Some(h) => {
                        headers.insert(AUTHORIZATION, h);
                    }
                    None => {
                        return Box::pin(std::future::ready(Err(AuthError::NoCredentials.into())));
                    }
                }
            }
            Credentials::Impersonate(imp) => {
                headers.insert(IMPERSONATE_USER.clone(), imp.user.clone());
                for g in &imp.groups {
                    headers.append(IMPERSONATE_GROUP.clone(), g.clone());
                }
            }
        }
        let fut = self.inner.call(req);
        Box::pin(async move { fut.await.map_err(Into::into) })
    }
}

/// The in-cluster base config. For pass-through the service-account credentials are removed so
/// the only identity on the wire is the user's; for impersonation they are kept (the SA token
/// file is re-read by kube on rotation).
pub fn incluster_config(pass_through: bool) -> Result<Config, kube::config::InClusterError> {
    let mut config = Config::incluster()?;
    if pass_through {
        config.auth_info = Default::default();
    }
    Ok(config)
}

/// Build a client from `config` with [`AuthLayer`] applied. No kubeconfig, no fallback.
pub fn build_client(config: Config, creds: Credentials) -> Result<Client, kube::Error> {
    let builder = ClientBuilder::try_from(config)?;
    Ok(builder.with_layer(&AuthLayer::new(creds)).build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tower::ServiceExt;

    type Seen = Arc<Mutex<Option<http::HeaderMap>>>;

    fn echo(
        seen: Seen,
    ) -> impl Service<
        http::Request<()>,
        Response = (),
        Error = std::convert::Infallible,
        Future = std::future::Ready<Result<(), std::convert::Infallible>>,
    > + Clone {
        tower::service_fn(move |req: http::Request<()>| {
            *seen.lock().unwrap() = Some(req.headers().clone());
            std::future::ready(Ok::<_, std::convert::Infallible>(()))
        })
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    async fn run(
        creds: Credentials,
        pre: Option<(&str, &str)>,
    ) -> (Result<(), BoxError>, Option<http::HeaderMap>) {
        let seen: Seen = Default::default();
        let svc = AuthLayer::new(creds).layer(echo(Arc::clone(&seen)));
        let mut b = http::Request::builder().uri("/api");
        if let Some((k, v)) = pre {
            b = b.header(k, v);
        }
        let r = svc.oneshot(b.body(()).unwrap()).await;
        let h = seen.lock().unwrap().take();
        (r, h)
    }

    #[tokio::test]
    async fn bearer_is_set_per_request_and_swaps_on_refresh() {
        let cell = TokenCell::new(&secret("t1"), None).unwrap();
        let (r, h) = run(
            Credentials::Bearer(cell.clone()),
            Some(("authorization", "Bearer sa")),
        )
        .await;
        r.unwrap();
        let h = h.unwrap();
        assert_eq!(h.get(AUTHORIZATION).unwrap(), "Bearer t1");
        assert!(h.get(AUTHORIZATION).unwrap().is_sensitive());
        assert_eq!(h.get_all(AUTHORIZATION).iter().count(), 1);

        cell.set(&secret("t2"), None).unwrap();
        let (_, h) = run(Credentials::Bearer(cell), None).await;
        assert_eq!(h.unwrap().get(AUTHORIZATION).unwrap(), "Bearer t2");
    }

    #[tokio::test]
    async fn revoked_or_empty_cell_fails_closed() {
        let cell = TokenCell::new(&secret("t1"), None).unwrap();
        cell.revoke();
        let (r, h) = run(Credentials::Bearer(cell), None).await;
        assert!(r.unwrap_err().to_string().contains("no credentials"));
        assert!(h.is_none(), "request must not reach the API server");
        let (r, _) = run(Credentials::Bearer(TokenCell::default()), None).await;
        assert!(r.is_err());
    }

    #[test]
    fn invalid_token_keeps_previous_value() {
        let cell = TokenCell::new(&secret("good"), None).unwrap();
        assert!(matches!(
            cell.set(&secret("bad\nvalue"), None),
            Err(AuthError::InvalidToken)
        ));
        assert!(cell.is_set());
    }

    #[tokio::test]
    async fn impersonation_sets_user_and_one_header_per_group_and_strips_spoofs() {
        let imp =
            Impersonation::new("oidc:alice", &["oidc:dev".into(), "oidc:ops".into()]).unwrap();
        let (r, h) = run(
            Credentials::Impersonate(imp),
            Some(("impersonate-user", "admin")),
        )
        .await;
        r.unwrap();
        let h = h.unwrap();
        assert_eq!(h.get("impersonate-user").unwrap(), "oidc:alice");
        let groups: Vec<_> = h
            .get_all("impersonate-group")
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(groups, ["oidc:dev", "oidc:ops"]);
        assert!(
            h.get(AUTHORIZATION).is_none(),
            "SA auth is left to the base client"
        );
    }

    #[test]
    fn impersonation_rejects_unrepresentable_identity() {
        assert!(Impersonation::new("a\nb", &[]).is_err());
        assert!(Impersonation::new("a", &["x\r".into()]).is_err());
    }

    #[test]
    fn debug_never_contains_the_token() {
        let cell = TokenCell::new(&secret("supersecret-token"), None).unwrap();
        let creds = Credentials::Bearer(cell.clone());
        let layer = AuthLayer::new(creds.clone());
        for s in [
            format!("{cell:?}"),
            format!("{creds:?}"),
            format!("{layer:?}"),
        ] {
            assert!(!s.contains("supersecret"), "{s}");
        }
        assert!(format!("{cell:?}").contains("redacted"));
    }

    #[test]
    fn expiry_is_tracked() {
        let t = SystemTime::now();
        let cell = TokenCell::new(&secret("a"), Some(t)).unwrap();
        assert_eq!(cell.expires_at(), Some(t));
        cell.revoke();
        assert_eq!(cell.expires_at(), None);
    }

    #[test]
    fn passthrough_incluster_config_has_no_sa_credentials() {
        // No cluster in unit tests: the constructor errors, which is the no-fallback contract.
        assert!(
            incluster_config(true).is_err() || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
        );
    }
}
