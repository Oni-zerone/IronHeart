//! Google sign-in (OpenID Connect, authorization code flow with PKCE) and sessions.
//!
//! - `GET  /auth/google/login`: creates a pending login and redirects to Google.
//! - `GET  /auth/google/callback`: verifies the ID token, creates the user and the session.
//! - `POST /auth/logout`: ends the session.

use std::sync::Arc;

use anyhow::{Context, anyhow};
use axum::{
    Router,
    extract::{FromRef, FromRequestParts, OptionalFromRequestParts, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    AppState,
    clock::{DAY_MS, MINUTE_MS, now_ms},
    config::Config,
    db,
    error::AppResult,
};

const SESSION_COOKIE: &str = "ih_session";
const STATE_COOKIE: &str = "ih_oauth_state";
const SESSION_TTL_MS: i64 = 30 * DAY_MS;
const LOGIN_TTL_MS: i64 = 10 * MINUTE_MS;
const GOOGLE_ISSUER: &str = "https://accounts.google.com";

type GoogleClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[derive(Clone)]
pub struct Auth(Arc<AuthInner>);

struct AuthInner {
    client: GoogleClient,
    http: reqwest::Client,
    public_base_url: String,
    secure_cookies: bool,
}

impl Auth {
    /// Reads Google's OpenID configuration. Done once at startup.
    pub async fn discover(config: &Config) -> anyhow::Result<Self> {
        let http = reqwest::ClientBuilder::new()
            // Prevents SSRF attacks through redirects (recommended by openidconnect).
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let metadata =
            CoreProviderMetadata::discover_async(IssuerUrl::new(GOOGLE_ISSUER.into())?, &http)
                .await
                .context("Google OpenID configuration unreachable")?;
        let redirect = format!("{}/auth/google/callback", config.public_base_url);
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(config.google_client_id.clone()),
            Some(ClientSecret::new(config.google_client_secret.clone())),
        )
        .set_redirect_uri(RedirectUrl::new(redirect)?);

        Ok(Self(Arc::new(AuthInner {
            client,
            http,
            public_base_url: config.public_base_url.clone(),
            secure_cookies: config.secure_cookies(),
        })))
    }

    /// Fake client for tests: no calls to Google.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        use openidconnect::{
            AuthUrl, EmptyAdditionalProviderMetadata, JsonWebKeySetUrl, ResponseTypes,
            core::{CoreJsonWebKeySet, CoreJwsSigningAlgorithm, CoreResponseType, CoreSubjectIdentifierType},
        };
        let metadata = CoreProviderMetadata::new(
            IssuerUrl::new("https://issuer.test".into()).unwrap(),
            AuthUrl::new("https://issuer.test/auth".into()).unwrap(),
            JsonWebKeySetUrl::new("https://issuer.test/jwks".into()).unwrap(),
            vec![ResponseTypes::new(vec![CoreResponseType::Code])],
            vec![CoreSubjectIdentifierType::Public],
            vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            EmptyAdditionalProviderMetadata {},
        )
        .set_jwks(CoreJsonWebKeySet::new(vec![]));
        let client = CoreClient::from_provider_metadata(metadata, ClientId::new("test".into()), None)
            .set_redirect_uri(RedirectUrl::new("http://localhost/auth/google/callback".into()).unwrap());
        Self(Arc::new(AuthInner {
            client,
            http: reqwest::Client::new(),
            public_base_url: "http://localhost".into(),
            secure_cookies: false,
        }))
    }

    fn home(&self) -> String {
        format!("{}/", self.0.public_base_url)
    }

    fn cookie(&self, name: &'static str, value: String, path: &'static str) -> Cookie<'static> {
        Cookie::build((name, value))
            .path(path)
            .http_only(true)
            .secure(self.0.secure_cookies)
            .same_site(SameSite::Lax)
            .build()
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/google/login", get(login))
        .route("/auth/google/callback", get(callback))
        .route("/auth/logout", post(logout))
}

async fn login(State(state): State<AppState>, jar: CookieJar) -> AppResult<Response> {
    let auth = &state.auth.0;
    let now = now_ms();
    db::delete_oauth_flows_before(&state.db, now - LOGIN_TTL_MS).await?;

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, csrf, nonce) = auth
        .client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let flow = db::OAuthFlow {
        nonce: nonce.secret().clone(),
        pkce_verifier: pkce_verifier.secret().clone(),
    };
    db::insert_oauth_flow(&state.db, csrf.secret(), &flow, now).await?;

    // Binds the login to this browser: the callback must arrive with the same `state`.
    let mut cookie = state.auth.cookie(STATE_COOKIE, csrf.secret().clone(), "/auth");
    cookie.set_max_age(time::Duration::milliseconds(LOGIN_TTL_MS));
    Ok((jar.add(cookie), Redirect::to(url.as_str())).into_response())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<CallbackQuery>,
) -> AppResult<Response> {
    // Read it before removing it: after `remove` the jar no longer returns it.
    let cookie_state = jar_value(&jar, STATE_COOKIE);
    let jar = jar.remove(state.auth.cookie(STATE_COOKIE, String::new(), "/auth"));
    let redirect_with = |outcome: &str| {
        let url = format!("{}?login={outcome}", state.auth.home());
        (jar.clone(), Redirect::to(&url)).into_response()
    };
    let failed = || redirect_with("failed");

    if let Some(error) = &query.error {
        tracing::info!(%error, "login cancelled by Google");
        return Ok(failed());
    }
    let (Some(code), Some(returned_state)) = (query.code, query.state) else {
        return Ok(failed());
    };
    if cookie_state.as_deref() != Some(returned_state.as_str()) {
        tracing::warn!("login: state does not match the cookie");
        return Ok(failed());
    }
    let now = now_ms();
    let Some(flow) = db::take_oauth_flow(&state.db, &returned_state, now - LOGIN_TTL_MS).await?
    else {
        tracing::warn!("login: unknown or expired state");
        return Ok(failed());
    };

    let identity = match exchange(&state.auth, code, flow).await {
        Ok(identity) => identity,
        Err(err) => {
            tracing::warn!(err = ?err, "login: Google verification failed");
            return Ok(failed());
        }
    };

    let email = identity.email.to_lowercase();
    let is_master = email == state.admin_email.as_ref();
    let Some(user) = db::login_google_user(
        &state.db,
        &identity.sub,
        &email,
        identity.name.as_deref(),
        is_master,
        now,
    )
    .await?
    else {
        tracing::info!("login rejected: account without an invite");
        return Ok(redirect_with("denied"));
    };

    let token = hex::encode(rand::random::<[u8; 32]>());
    db::insert_session(&state.db, &hash_token(&token), &user.id, now, now + SESSION_TTL_MS).await?;
    db::delete_expired_sessions(&state.db, now).await?;
    tracing::info!(user_id = %user.id, "signed in");

    let mut cookie = state.auth.cookie(SESSION_COOKIE, token, "/");
    cookie.set_max_age(time::Duration::milliseconds(SESSION_TTL_MS));
    Ok((jar.add(cookie), Redirect::to(&state.auth.home())).into_response())
}

struct GoogleIdentity {
    sub: String,
    email: String,
    name: Option<String>,
}

async fn exchange(auth: &Auth, code: String, flow: db::OAuthFlow) -> anyhow::Result<GoogleIdentity> {
    let auth = &auth.0;
    let token = auth
        .client
        .exchange_code(AuthorizationCode::new(code))?
        .set_pkce_verifier(PkceCodeVerifier::new(flow.pkce_verifier))
        .request_async(&auth.http)
        .await?;
    let id_token = token.id_token().ok_or_else(|| anyhow!("Google did not return the ID token"))?;
    let claims = id_token.claims(&auth.client.id_token_verifier(), &Nonce::new(flow.nonce))?;

    if claims.email_verified() != Some(true) {
        anyhow::bail!("email not verified by Google");
    }
    let email = claims.email().ok_or_else(|| anyhow!("email missing from the ID token"))?;
    Ok(GoogleIdentity {
        sub: claims.subject().to_string(),
        email: email.to_string(),
        name: claims.name().and_then(|n| n.get(None)).map(|n| n.to_string()),
    })
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> AppResult<Response> {
    if let Some(token) = jar_value(&jar, SESSION_COOKIE) {
        db::delete_session(&state.db, &hash_token(&token)).await?;
    }
    let jar = jar.remove(state.auth.cookie(SESSION_COOKIE, String::new(), "/"));
    Ok((jar, StatusCode::NO_CONTENT).into_response())
}

fn jar_value(jar: &CookieJar, name: &str) -> Option<String> {
    jar.get(name).map(|c| c.value().to_owned())
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Master user (`ADMIN_EMAIL`) of the current session: 401 without a session, 403 otherwise.
pub struct MasterUser(#[allow(dead_code)] pub db::User);

impl<S> FromRequestParts<S> for MasterUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let CurrentUser(user) = <CurrentUser as FromRequestParts<S>>::from_request_parts(parts, state).await?;
        let state = AppState::from_ref(state);
        if user.email == state.admin_email.as_ref() {
            Ok(Self(user))
        } else {
            Err(StatusCode::FORBIDDEN.into_response())
        }
    }
}

/// Admin user of the current session: 401 without a session, 403 if not an admin.
pub struct AdminUser(pub db::User);

impl<S> FromRequestParts<S> for AdminUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let CurrentUser(user) = <CurrentUser as FromRequestParts<S>>::from_request_parts(parts, state).await?;
        if user.is_admin() { Ok(Self(user)) } else { Err(StatusCode::FORBIDDEN.into_response()) }
    }
}

/// User of the current session. In handlers it answers 401 if missing;
/// as `Option<CurrentUser>` it is `None`.
pub struct CurrentUser(pub db::User);

// `Response` is the extractors' rejection type: boxing it gains nothing.
#[allow(clippy::result_large_err)]
async fn current_user(parts: &Parts, state: &AppState) -> Result<Option<CurrentUser>, Response> {
    let jar = CookieJar::from_headers(&parts.headers);
    let Some(token) = jar_value(&jar, SESSION_COOKIE) else {
        return Ok(None);
    };
    db::session_user(&state.db, &hash_token(&token), now_ms())
        .await
        .map(|user| user.map(CurrentUser))
        .map_err(|err| crate::error::AppError::from(err).into_response())
}

impl<S> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        current_user(parts, &AppState::from_ref(state))
            .await?
            .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())
    }
}

impl<S> OptionalFromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        current_user(parts, &AppState::from_ref(state)).await
    }
}
