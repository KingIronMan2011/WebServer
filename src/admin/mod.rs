//! Authenticated management API and its local bootstrap workflow.
//!
//! This module intentionally owns authentication rather than delegating it to a
//! second runtime. The public HTTP server stays small; only the dedicated admin
//! listener exposes these routes.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    extract::{ConnectInfo, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use sqlx_core::{
    pool::{Pool, PoolOptions},
    row::Row,
};
use sqlx_sqlite::{Sqlite, SqliteConnectOptions, SqliteJournalMode};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::{Config, RouteConfig, RouteTarget},
    error::{Error, Result},
};

// The `sqlx` facade locks optional MySQL code (and its unpatched RSA crate)
// even for SQLite-only builds. Import only the two driver crates we execute.
mod sqlx {
    pub use sqlx_core::{Error, query::query, query_scalar::query_scalar};
}

type SqlitePool = Pool<Sqlite>;
type SqlitePoolOptions = PoolOptions<Sqlite>;
use tower::Service;
use tower_http::set_header::SetResponseHeaderLayer;

pub(crate) const SESSION_COOKIE: &str = "__Host-webserver_admin";
const SETUP_GRANT_LIFETIME: Duration = Duration::from_secs(15 * 60);
const SESSION_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
const API_VERSION: u32 = 1;
const ADMIN_MAX_BODY_BYTES: usize = 64 * 1024;
const ADMIN_MAX_CONNECTIONS: usize = 256;
const MAX_AUTH_ATTEMPT_BUCKETS: usize = 16 * 1024;
const MAX_USERNAME_BYTES: usize = 128;
const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_SETUP_CODE_BYTES: usize = 256;
const PASSWORD_WORKERS: usize = 4;
const MAX_SESSIONS_PER_USER: i64 = 16;
const MAX_AUDIT_ENTRIES: i64 = 100_000;

#[derive(Clone)]
pub struct AdminState {
    config: Arc<RwLock<Config>>,
    database: SqlitePool,
    attempts: Arc<Mutex<AttemptStore>>,
}

#[derive(Clone, Copy)]
struct AttemptBucket {
    failures: u8,
    retry_after: Instant,
    expires_at: Instant,
}

struct AttemptStore {
    buckets: HashMap<IpAddr, AttemptBucket>,
    last_pruned: Instant,
}

impl Default for AttemptStore {
    fn default() -> Self {
        Self {
            buckets: HashMap::new(),
            last_pruned: Instant::now(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Admin,
    Operator,
    Viewer,
}

impl Role {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    fn can_write(self) -> bool {
        matches!(self, Self::Admin | Self::Operator)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }
}

#[derive(Clone, Debug)]
struct Principal {
    user_id: String,
    role: Role,
}

#[derive(Debug, Serialize)]
struct ApiError {
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    fn response(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
    ) -> axum::response::Response {
        (status, Json(Self { code, message })).into_response()
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    setup_code: Option<String>,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    password_change_required: bool,
    passkey_enrolment_required: bool,
    role: &'static str,
}

#[derive(Debug, Deserialize)]
struct PasswordChangeRequest {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Serialize)]
struct SiteSummary {
    host: String,
    routes: usize,
}

#[derive(Debug, Deserialize)]
struct CreateSiteRequest {
    host: String,
    route: RouteConfig,
}

#[derive(Debug, Deserialize)]
struct RouteQuery {
    path_prefix: String,
}

#[derive(Debug, Serialize)]
struct CertificateSummary {
    hosts: Vec<String>,
    source: &'static str,
    expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuditEntry {
    created_at: i64,
    user_id: Option<String>,
    source_ip: String,
    action: String,
    target: String,
    success: bool,
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct UpdateRoleRequest {
    role: String,
}

/// Starts a TLS-only admin listener. It is only invoked after local `web-init`
/// created an account and enabled the section in the server configuration.
pub async fn spawn(
    config: Arc<RwLock<Config>>,
    tls: Arc<crate::tls::TlsManager>,
    shutdown: CancellationToken,
) -> Result<tokio::task::JoinHandle<()>> {
    let settings = config.read().await.admin.clone();
    if !settings.enabled {
        return Err(Error::Config(
            "admin listener requested while disabled".into(),
        ));
    }
    let database = open_database(&settings.database).await?;
    // Build the dummy verifier before accepting traffic so an unknown account
    // takes the same password-verification path without a first-request spike.
    let _ = dummy_password_hash();
    let state = AdminState {
        config,
        database,
        attempts: Arc::new(Mutex::new(AttemptStore::default())),
    };
    let connection_slots = Arc::new(tokio::sync::Semaphore::new(ADMIN_MAX_CONNECTIONS));
    let max_header_bytes = state.config.read().await.server.max_header_bytes;
    let listener = tokio::net::TcpListener::bind(settings.bind).await?;
    tracing::info!(address = %settings.bind, host = ?settings.host, "listening for HTTPS admin API");
    Ok(tokio::spawn(async move {
        loop {
            let (stream, peer) = match tokio::select! {
                _ = shutdown.cancelled() => return,
                accepted = listener.accept() => accepted,
            } {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(%error, "failed to accept admin connection");
                    continue;
                }
            };
            let Ok(connection_slot) = Arc::clone(&connection_slots).try_acquire_owned() else {
                tracing::warn!(%peer, "admin connection limit reached");
                continue;
            };
            let acceptor = tls.acceptor();
            let app = router(state.clone());
            tokio::spawn(async move {
                let _connection_slot = connection_slot;
                let stream =
                    match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream))
                        .await
                    {
                        Ok(Ok(stream)) => stream,
                        Ok(Err(error)) => {
                            tracing::debug!(%peer, %error, "admin TLS handshake failed");
                            return;
                        }
                        Err(_) => {
                            tracing::debug!(%peer, "admin TLS handshake timed out");
                            return;
                        }
                    };
                let io = hyper_util::rt::TokioIo::new(stream);
                let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();
                let service = match make_service.call(peer).await {
                    Ok(service) => service,
                    Err(never) => match never {},
                };
                let mut builder = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                );
                builder
                    .http1()
                    .max_buf_size(max_header_bytes)
                    .header_read_timeout(Duration::from_secs(10))
                    .timer(hyper_util::rt::TokioTimer::new());
                builder
                    .http2()
                    .max_header_list_size(u32::try_from(max_header_bytes).unwrap_or(u32::MAX));
                if let Err(error) = builder
                    .serve_connection(io, hyper_util::service::TowerToHyperService::new(service))
                    .await
                {
                    tracing::debug!(%peer, %error, "admin connection closed");
                }
            });
        }
    }))
}

fn router(state: AdminState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(current_user))
        .route("/api/v1/auth/password", post(change_password))
        .route("/api/v1/users", get(list_users).post(create_user))
        .route("/api/v1/users/{username}/role", post(update_user_role))
        .route("/api/v1/sites", get(list_sites).post(create_site))
        .route("/api/v1/sites/{host}", delete(delete_site))
        .route(
            "/api/v1/sites/{host}/routes",
            get(list_routes).post(create_route),
        )
        .route("/api/v1/sites/{host}/routes", delete(delete_route))
        .route("/api/v1/upstreams", get(list_upstreams))
        .route("/api/v1/certificates", get(list_certificates))
        .route("/api/v1/logs", get(list_audit_log))
        .route("/api/v1/metrics", get(metrics))
        .route("/api/v1/observability", get(observability))
        .route("/api/v1/openapi.json", get(openapi))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-webserver-api-version"),
            HeaderValue::from_static("1"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(DefaultBodyLimit::max(ADMIN_MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_admin_host,
        ))
        .with_state(state)
}

async fn enforce_admin_host(
    State(state): State<AdminState>,
    request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let expected = state
        .config
        .read()
        .await
        .admin
        .host
        .as_deref()
        .map(crate::config::normalise_host);
    let actual = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(crate::http::request::authority_host)
        .map(|host| crate::config::normalise_host(&host))
        .or_else(|| {
            request
                .uri()
                .authority()
                .and_then(|authority| crate::http::request::authority_host(authority.as_str()))
                .map(|host| crate::config::normalise_host(&host))
        });
    if expected.is_none() || actual != expected {
        return ApiError::response(
            StatusCode::MISDIRECTED_REQUEST,
            "invalid_host",
            "request host does not match the admin host",
        );
    }
    next.run(request).await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn login(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    Json(input): Json<LoginRequest>,
) -> axum::response::Response {
    if !allow_attempt(&state, peer.ip()).await {
        return ApiError::response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "try again later",
        );
    }
    if input.username.trim().is_empty()
        || input.username.len() > MAX_USERNAME_BYTES
        || input.password.is_empty()
        || input.password.len() > MAX_PASSWORD_BYTES
        || input
            .setup_code
            .as_ref()
            .is_some_and(|code| code.len() > MAX_SETUP_CODE_BYTES)
    {
        fail_attempt(&state, peer.ip()).await;
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "login failed",
        );
    }
    let row = match sqlx::query(
        "SELECT id, password_hash, password_change_required, role FROM admin_users WHERE username = ?",
    )
    .bind(&input.username)
    .fetch_optional(&state.database)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            tracing::error!(%error, "admin login query failed");
            return ApiError::response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            );
        }
    };
    let verification_hash = row
        .as_ref()
        .map(|row| row.get::<String, _>("password_hash"))
        .unwrap_or_else(|| dummy_password_hash().to_owned());
    let Some(password_matches) = verify_password_bounded(input.password, verification_hash).await
    else {
        return ApiError::response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "try again later",
        );
    };
    let Some(row) = row else {
        fail_attempt(&state, peer.ip()).await;
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "login failed",
        );
    };
    if !password_matches {
        fail_attempt(&state, peer.ip()).await;
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "login failed",
        );
    }
    let user_id: String = row.get("id");
    let Some(role) = Role::parse(row.get::<String, _>("role").as_str()) else {
        tracing::error!(%user_id, "admin account has an invalid role");
        return ApiError::response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        );
    };
    let password_change_required: i64 = row.get("password_change_required");
    if password_change_required != 0
        && !consume_setup_grant(&state.database, &user_id, input.setup_code.as_deref()).await
    {
        fail_attempt(&state, peer.ip()).await;
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "login failed",
        );
    }
    clear_attempt(&state, peer.ip()).await;
    let token = Uuid::new_v4().to_string() + &Uuid::new_v4().to_string();
    let expires = unix_after(SESSION_LIFETIME);
    let _ = sqlx::query("DELETE FROM admin_sessions WHERE expires_at < ?")
        .bind(unix_after(Duration::ZERO))
        .execute(&state.database)
        .await;
    if let Err(error) =
        sqlx::query("INSERT INTO admin_sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(secret_hash(&token))
            .bind(&user_id)
            .bind(expires)
            .execute(&state.database)
            .await
    {
        tracing::error!(%error, "could not create admin session");
        return ApiError::response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        );
    }
    if let Err(error) = sqlx::query(
        "DELETE FROM admin_sessions WHERE rowid IN (SELECT rowid FROM admin_sessions WHERE user_id = ? ORDER BY expires_at DESC, rowid DESC LIMIT -1 OFFSET ?)",
    )
    .bind(&user_id)
    .bind(MAX_SESSIONS_PER_USER)
    .execute(&state.database)
    .await
    {
        tracing::error!(%error, "could not enforce the per-user session limit");
        return ApiError::response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        );
    }
    audit(
        &state.database,
        Some(&user_id),
        peer.ip(),
        "auth.login",
        "session",
        true,
    )
    .await;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(time::Duration::hours(8));
    (
        jar.add(cookie),
        Json(LoginResponse {
            password_change_required: password_change_required != 0,
            passkey_enrolment_required: password_change_required != 0,
            role: role.as_str(),
        }),
    )
        .into_response()
}

async fn current_user(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    match authenticated_principal(&state.database, &jar, &headers).await {
        Some(principal) => {
            Json(serde_json::json!({ "role": principal.role.as_str() })).into_response()
        }
        None => ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        ),
    }
}

async fn list_users(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    let Some(principal) = authenticated_principal(&state.database, &jar, &headers).await else {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    };
    if principal.role != Role::Admin {
        return ApiError::response(StatusCode::FORBIDDEN, "forbidden", "admin role required");
    }
    match sqlx::query("SELECT username, role FROM admin_users ORDER BY username").fetch_all(&state.database).await {
        Ok(rows) => Json(rows.into_iter().map(|row| serde_json::json!({ "username": row.get::<String, _>("username"), "role": row.get::<String, _>("role") })).collect::<Vec<_>>()).into_response(),
        Err(error) => { tracing::error!(%error, "could not list admin users"); ApiError::response(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", "internal server error") }
    }
}

async fn create_user(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(input): Json<CreateUserRequest>,
) -> axum::response::Response {
    let Some(principal) = authenticated_principal(&state.database, &jar, &headers).await else {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    };
    if principal.role != Role::Admin {
        return ApiError::response(StatusCode::FORBIDDEN, "forbidden", "admin role required");
    }
    let Some(role) = Role::parse(&input.role) else {
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "role must be admin, operator, or viewer",
        );
    };
    if input.username.trim().is_empty()
        || input.username.len() > MAX_USERNAME_BYTES
        || input.password.len() < 16
        || input.password.len() > MAX_PASSWORD_BYTES
    {
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_user",
            "username or password is invalid",
        );
    }
    let Some(hash) = hash_password_bounded(input.password).await else {
        return ApiError::response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "try again later",
        );
    };
    let hash = match hash {
        Ok(hash) => hash,
        Err(_) => {
            return ApiError::response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            );
        }
    };
    let result = sqlx::query("INSERT INTO admin_users (id, username, password_hash, password_change_required, role) VALUES (?, ?, ?, 1, ?)")
        .bind(Uuid::new_v4().to_string()).bind(input.username.trim()).bind(hash).bind(role.as_str()).execute(&state.database).await;
    if let Err(error) = result {
        tracing::warn!(%error, "rejected admin user creation");
        return ApiError::response(
            StatusCode::CONFLICT,
            "user_exists",
            "user could not be created",
        );
    }
    audit(
        &state.database,
        Some(&principal.user_id),
        peer.ip(),
        "user.created",
        "user",
        true,
    )
    .await;
    StatusCode::CREATED.into_response()
}

async fn update_user_role(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    AxumPath(username): AxumPath<String>,
    Json(input): Json<UpdateRoleRequest>,
) -> axum::response::Response {
    let Some(principal) = authenticated_principal(&state.database, &jar, &headers).await else {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    };
    if principal.role != Role::Admin {
        return ApiError::response(StatusCode::FORBIDDEN, "forbidden", "admin role required");
    }
    let Some(role) = Role::parse(&input.role) else {
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "role must be admin, operator, or viewer",
        );
    };
    if username.is_empty() {
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_user",
            "username is invalid",
        );
    }
    match sqlx::query(
        "UPDATE admin_users SET role = ? WHERE username = ? AND (? = 'admin' OR role != 'admin' OR (SELECT COUNT(*) FROM admin_users WHERE role = 'admin') > 1)",
    )
        .bind(role.as_str())
        .bind(&username)
        .bind(role.as_str())
        .execute(&state.database)
        .await
    {
        Ok(result) if result.rows_affected() == 1 => {
            audit(
                &state.database,
                Some(&principal.user_id),
                peer.ip(),
                "user.role_changed",
                &username,
                true,
            )
            .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(_) => {
            let exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM admin_users WHERE username = ?",
            )
            .bind(&username)
            .fetch_one(&state.database)
            .await
            .unwrap_or_default()
                != 0;
            if exists {
                ApiError::response(
                    StatusCode::CONFLICT,
                    "last_admin",
                    "the final admin account cannot be demoted",
                )
            } else {
                ApiError::response(
                    StatusCode::NOT_FOUND,
                    "user_not_found",
                    "user does not exist",
                )
            }
        }
        Err(error) => {
            tracing::error!(%error, "could not update user role");
            ApiError::response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            )
        }
    }
}

async fn logout(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(token) = session_token(&jar, &headers) {
        let _ = sqlx::query("DELETE FROM admin_sessions WHERE token_hash = ?")
            .bind(secret_hash(token))
            .execute(&state.database)
            .await;
    }
    (jar.remove(expired_session_cookie()), StatusCode::NO_CONTENT).into_response()
}

async fn change_password(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(input): Json<PasswordChangeRequest>,
) -> axum::response::Response {
    if input.current_password.is_empty()
        || input.current_password.len() > MAX_PASSWORD_BYTES
        || input.new_password.len() < 16
        || input.new_password.len() > MAX_PASSWORD_BYTES
    {
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "weak_password",
            "password must be at least 16 characters",
        );
    }
    let Some(user_id) = authenticated_user(&state.database, &jar, &headers).await else {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    };
    let row = match sqlx::query("SELECT password_hash FROM admin_users WHERE id = ?")
        .bind(&user_id)
        .fetch_one(&state.database)
        .await
    {
        Ok(row) => row,
        Err(_) => {
            return ApiError::response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "authentication required",
            );
        }
    };
    let current: String = row.get("password_hash");
    let Some(password_matches) = verify_password_bounded(input.current_password, current).await
    else {
        return ApiError::response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "try again later",
        );
    };
    if !password_matches {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "login failed",
        );
    }
    let Some(hash) = hash_password_bounded(input.new_password).await else {
        return ApiError::response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "try again later",
        );
    };
    let hash = match hash {
        Ok(hash) => hash,
        Err(_) => {
            return ApiError::response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            );
        }
    };
    if let Err(error) = sqlx::query(
        "UPDATE admin_users SET password_hash = ?, password_change_required = 0 WHERE id = ?",
    )
    .bind(hash)
    .bind(&user_id)
    .execute(&state.database)
    .await
    {
        tracing::error!(%error, "password update failed");
        return ApiError::response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        );
    }
    // A password rotation is a credential-recovery boundary. Revoke every
    // outstanding bearer/cookie session so a stolen token cannot survive it.
    if let Err(error) = sqlx::query("DELETE FROM admin_sessions WHERE user_id = ?")
        .bind(&user_id)
        .execute(&state.database)
        .await
    {
        tracing::error!(%error, "session revocation after password update failed");
        return ApiError::response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        );
    }
    audit(
        &state.database,
        Some(&user_id),
        peer.ip(),
        "auth.password_changed",
        "user",
        true,
    )
    .await;
    (jar.remove(expired_session_cookie()), StatusCode::NO_CONTENT).into_response()
}

async fn list_sites(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    let config = state.config.read().await;
    Json(
        config
            .sites
            .iter()
            .map(|site| SiteSummary {
                host: site.host.clone(),
                routes: site.routes.len(),
            })
            .collect::<Vec<_>>(),
    )
    .into_response()
}

async fn create_site(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(input): Json<CreateSiteRequest>,
) -> axum::response::Response {
    let principal = match write_principal(&state.database, &jar, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let mut active = state.config.write().await;
    let mut config = active.clone();
    if let Err(error) = crate::management::create_site(&mut config, input.host, input.route) {
        tracing::warn!(%error, "rejected admin site creation");
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_site",
            "site or initial route is invalid",
        );
    }
    *active = config;
    audit(
        &state.database,
        Some(&principal.user_id),
        peer.ip(),
        "site.created",
        "site",
        true,
    )
    .await;
    StatusCode::CREATED.into_response()
}

async fn list_routes(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
    AxumPath(host): AxumPath<String>,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    let config = state.config.read().await;
    match config
        .sites
        .iter()
        .find(|site| site.host.eq_ignore_ascii_case(&host))
    {
        Some(site) => Json(site.routes.clone()).into_response(),
        None => ApiError::response(
            StatusCode::NOT_FOUND,
            "site_not_found",
            "site does not exist",
        ),
    }
}

async fn create_route(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    AxumPath(host): AxumPath<String>,
    Json(route): Json<RouteConfig>,
) -> axum::response::Response {
    let principal = match write_principal(&state.database, &jar, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let mut active = state.config.write().await;
    let mut config = active.clone();
    if let Err(error) = crate::management::add_route(&mut config, &host, route) {
        tracing::warn!(%error, "rejected admin route creation");
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_route",
            "route configuration is invalid",
        );
    }
    *active = config;
    audit(
        &state.database,
        Some(&principal.user_id),
        peer.ip(),
        "route.created",
        &host,
        true,
    )
    .await;
    StatusCode::CREATED.into_response()
}

async fn delete_route(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    AxumPath(host): AxumPath<String>,
    Query(query): Query<RouteQuery>,
) -> axum::response::Response {
    let principal = match write_principal(&state.database, &jar, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let mut active = state.config.write().await;
    let mut config = active.clone();
    if let Err(error) = crate::management::remove_route(&mut config, &host, &query.path_prefix) {
        tracing::warn!(%error, "rejected admin route deletion");
        return ApiError::response(
            StatusCode::BAD_REQUEST,
            "invalid_route",
            "route does not exist or would invalidate the site",
        );
    }
    *active = config;
    audit(
        &state.database,
        Some(&principal.user_id),
        peer.ip(),
        "route.deleted",
        &host,
        true,
    )
    .await;
    StatusCode::NO_CONTENT.into_response()
}

async fn list_upstreams(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    let config = state.config.read().await;
    let mut urls = std::collections::BTreeSet::new();
    for site in &config.sites {
        for route in &site.routes {
            if let RouteTarget::Proxy {
                upstream,
                upstreams,
                ..
            } = &route.target
            {
                for (url, _) in crate::config::proxy_upstreams(upstream.as_deref(), upstreams) {
                    urls.insert(url.to_owned());
                }
            }
        }
    }
    Json(
        urls.into_iter()
            .map(|url| crate::handlers::reverse_proxy::status(&url))
            .collect::<Vec<_>>(),
    )
    .into_response()
}

async fn list_certificates(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    let config = state.config.read().await;
    let mut certificates = config
        .tls
        .certificates
        .iter()
        .map(|certificate| CertificateSummary {
            hosts: certificate.hosts.clone(),
            source: "local",
            expires_at: None,
        })
        .collect::<Vec<_>>();
    let local_hosts = certificates
        .iter()
        .flat_map(|certificate| certificate.hosts.iter().cloned())
        .collect::<std::collections::HashSet<_>>();
    for host in config
        .sites
        .iter()
        .map(|site| &site.host)
        .chain(config.admin.host.iter())
    {
        if !local_hosts.contains(host) {
            certificates.push(CertificateSummary {
                hosts: vec![host.clone()],
                source: "acme",
                expires_at: None,
            });
        }
    }
    Json(certificates).into_response()
}

async fn list_audit_log(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    match sqlx::query("SELECT created_at, user_id, source_ip, action, target, success FROM admin_audit_log ORDER BY id DESC LIMIT 200").fetch_all(&state.database).await {
        Ok(rows) => Json(rows.into_iter().map(|row| AuditEntry { created_at: row.get("created_at"), user_id: row.get("user_id"), source_ip: row.get("source_ip"), action: row.get("action"), target: row.get("target"), success: row.get::<i64, _>("success") != 0 }).collect::<Vec<_>>()).into_response(),
        Err(error) => { tracing::error!(%error, "could not read admin audit log"); ApiError::response(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", "internal server error") }
    }
}

async fn delete_site(
    State(state): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    headers: HeaderMap,
    AxumPath(host): AxumPath<String>,
) -> axum::response::Response {
    let principal = match write_principal(&state.database, &jar, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let mut config = state.config.write().await;
    if let Err(error) = crate::management::remove_site(&mut config, &host) {
        tracing::warn!(%error, "rejected admin site deletion");
        return ApiError::response(
            StatusCode::NOT_FOUND,
            "site_not_found",
            "site does not exist",
        );
    }
    audit(
        &state.database,
        Some(&principal.user_id),
        peer.ip(),
        "site.deleted",
        "site",
        true,
    )
    .await;
    StatusCode::NO_CONTENT.into_response()
}

async fn metrics(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    Json(serde_json::json!({ "prometheus": crate::observability::metrics::prometheus() }))
        .into_response()
}

async fn observability(
    State(state): State<AdminState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> axum::response::Response {
    if authenticated_user(&state.database, &jar, &headers)
        .await
        .is_none()
    {
        return ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        );
    }
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .ok()
        .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok());
    let tracing_enabled = endpoint.is_some();
    let endpoint = endpoint.as_deref().and_then(public_endpoint);
    let config = state.config.read().await;
    Json(serde_json::json!({
        "tracing": {
            "enabled": tracing_enabled,
            "exporter": if tracing_enabled { "otlp" } else { "none" },
            "endpoint": endpoint,
        },
        "prometheus": {
            "enabled": config.server.metrics_path.is_some(),
            "path": config.server.metrics_path.clone(),
        }
    }))
    .into_response()
}

fn public_endpoint(value: &str) -> Option<String> {
    let uri: hyper::Uri = value.parse().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?.as_str();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    Some(format!("{scheme}://{authority}"))
}

async fn openapi() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "Webserver Admin API", "version": "v1", "description": "TLS-only management API. Browser clients authenticate with an HttpOnly session cookie; automation may use a bearer session token." },
        "components": { "securitySchemes": { "adminSession": { "type": "apiKey", "in": "cookie", "name": SESSION_COOKIE }, "bearerSession": { "type": "http", "scheme": "bearer" } } },
        "paths": {
            "/api/v1/health": { "get": { "summary": "Service health", "responses": { "200": { "description": "Service status" } } } },
            "/api/v1/auth/login": { "post": { "summary": "Create an authenticated session", "responses": { "200": { "description": "Session cookie set" }, "401": { "description": "Invalid credentials or setup code" }, "429": { "description": "Login backoff active" } } } },
            "/api/v1/auth/logout": { "post": { "summary": "Revoke current session", "security": [{ "adminSession": [] }], "responses": { "204": { "description": "Session revoked" } } } },
            "/api/v1/auth/password": { "post": { "summary": "Change the current password", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "204": { "description": "Password changed" } } } },
            "/api/v1/sites": { "get": { "summary": "List sites", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Configured sites" } } }, "post": { "summary": "Create a site and initial route", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "201": { "description": "Site created" }, "400": { "description": "Invalid configuration" } } } },
            "/api/v1/sites/{host}": { "delete": { "summary": "Delete a site", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "204": { "description": "Site deleted" }, "404": { "description": "Unknown site" } } } },
            "/api/v1/sites/{host}/routes": { "get": { "summary": "List routes for a site", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Routes" } } }, "post": { "summary": "Create a route", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "201": { "description": "Route created" } } }, "delete": { "summary": "Delete a route using path_prefix query parameter", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "204": { "description": "Route deleted" } } } },
            "/api/v1/upstreams": { "get": { "summary": "List configured upstreams and live health", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Upstream status" } } } },
            "/api/v1/certificates": { "get": { "summary": "List certificate sources and hosts", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Certificate metadata without private keys" } } } },
            "/api/v1/logs": { "get": { "summary": "Read recent management audit log", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Latest audit events" } } } },
            "/api/v1/metrics": { "get": { "summary": "Read Prometheus metrics as JSON", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Metrics snapshot" } } } }
            ,"/api/v1/observability": { "get": { "summary": "Read tracing and Prometheus configuration", "security": [{ "adminSession": [] }, { "bearerSession": [] }], "responses": { "200": { "description": "Observability status" } } } }
        }
    }))
}

pub async fn bootstrap(
    database_path: &Path,
    username: &str,
    password: &str,
    setup_code: &str,
) -> Result<()> {
    if username.trim().is_empty() || password.len() < 16 {
        return Err(Error::Config(
            "admin username or generated password is invalid".into(),
        ));
    }
    let database = open_database(database_path).await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
        .fetch_one(&database)
        .await
        .map_err(sql_error)?;
    if count != 0 {
        return Err(Error::Config(
            "an admin account already exists; use admin reset locally".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let password_hash = hash_password(password)?;
    sqlx::query("INSERT INTO admin_users (id, username, password_hash, password_change_required, role) VALUES (?, ?, ?, 1, 'admin')")
        .bind(&id).bind(username).bind(password_hash).execute(&database).await.map_err(sql_error)?;
    sqlx::query("INSERT INTO admin_setup_grants (code_hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(secret_hash(setup_code))
        .bind(&id)
        .bind(unix_after(SETUP_GRANT_LIFETIME))
        .execute(&database)
        .await
        .map_err(sql_error)?;
    audit(
        &database,
        Some(&id),
        "127.0.0.1".parse().expect("loopback address"),
        "admin.bootstrap",
        "admin",
        true,
    )
    .await;
    Ok(())
}

/// Records a successful management action performed through the local CLI.
/// The CLI is a first-class management client and writes to the same audit log
/// as the remote API.
pub async fn audit_local(database_path: &Path, action: &str, target: &str) -> Result<()> {
    let database = open_database(database_path).await?;
    audit(
        &database,
        None,
        "127.0.0.1".parse().expect("loopback address"),
        action,
        target,
        true,
    )
    .await;
    Ok(())
}

async fn open_database(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    restrict_database_file(path)?;
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Delete);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(sql_error)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS admin_schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)").execute(&pool).await.map_err(sql_error)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS admin_users (id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, password_change_required INTEGER NOT NULL DEFAULT 1, role TEXT NOT NULL DEFAULT 'admin' CHECK(role IN ('admin', 'operator', 'viewer')))").execute(&pool).await.map_err(sql_error)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS admin_setup_grants (code_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL, expires_at INTEGER NOT NULL)").execute(&pool).await.map_err(sql_error)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS admin_sessions (token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL, expires_at INTEGER NOT NULL)").execute(&pool).await.map_err(sql_error)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS admin_audit_log (id INTEGER PRIMARY KEY AUTOINCREMENT, created_at INTEGER NOT NULL, user_id TEXT, source_ip TEXT NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, success INTEGER NOT NULL)").execute(&pool).await.map_err(sql_error)?;
    let columns = sqlx::query("PRAGMA table_info(admin_users)")
        .fetch_all(&pool)
        .await
        .map_err(sql_error)?;
    if !columns
        .iter()
        .any(|column| column.get::<String, _>("name") == "role")
    {
        sqlx::query("ALTER TABLE admin_users ADD COLUMN role TEXT NOT NULL DEFAULT 'admin' CHECK(role IN ('admin', 'operator', 'viewer'))").execute(&pool).await.map_err(sql_error)?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO admin_schema_migrations (version, applied_at) VALUES (?, ?)",
    )
    .bind(API_VERSION as i64)
    .bind(unix_after(Duration::ZERO))
    .execute(&pool)
    .await
    .map_err(sql_error)?;
    Ok(pool)
}

#[cfg(unix)]
fn restrict_database_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_database_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| Error::Config(format!("password hashing failed: {error}")))
}

fn password_hash_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(PASSWORD_WORKERS))
}

fn dummy_password_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        hash_password("constant dummy password used only for timing equalisation")
            .expect("the fixed dummy password can be hashed")
    })
}

async fn verify_password_bounded(password: String, hash: String) -> Option<bool> {
    let _slot = password_hash_slots().try_acquire().ok()?;
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .ok()
}

async fn hash_password_bounded(password: String) -> Option<Result<String>> {
    let _slot = password_hash_slots().try_acquire().ok()?;
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .ok()
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .ok()
        .and_then(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .ok()
        })
        .is_some()
}
fn secret_hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(secret.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}
fn unix_after(duration: Duration) -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_add(duration.as_secs()) as i64
}
fn sql_error(error: sqlx::Error) -> Error {
    Error::Config(format!("admin database error: {error}"))
}

async fn consume_setup_grant(database: &SqlitePool, user_id: &str, code: Option<&str>) -> bool {
    let Some(code) = code else {
        return false;
    };
    let now = unix_after(Duration::ZERO);
    match sqlx::query(
        "DELETE FROM admin_setup_grants WHERE code_hash = ? AND user_id = ? AND expires_at >= ?",
    )
    .bind(secret_hash(code))
    .bind(user_id)
    .bind(now)
    .execute(database)
    .await
    {
        Ok(result) => result.rows_affected() == 1,
        Err(_) => false,
    }
}
fn session_token<'a>(jar: &'a CookieJar, headers: &'a HeaderMap) -> Option<&'a str> {
    jar.get(SESSION_COOKIE)
        .map(|cookie| cookie.value())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
}

fn expired_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, String::new()))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(time::Duration::seconds(0))
        .build()
}
async fn authenticated_user(
    database: &SqlitePool,
    jar: &CookieJar,
    headers: &HeaderMap,
) -> Option<String> {
    let token = session_token(jar, headers)?;
    sqlx::query_scalar(
        "SELECT user_id FROM admin_sessions WHERE token_hash = ? AND expires_at >= ?",
    )
    .bind(secret_hash(token))
    .bind(unix_after(Duration::ZERO))
    .fetch_optional(database)
    .await
    .ok()
    .flatten()
}

async fn authenticated_principal(
    database: &SqlitePool,
    jar: &CookieJar,
    headers: &HeaderMap,
) -> Option<Principal> {
    let token = session_token(jar, headers)?;
    let row = sqlx::query(
        "SELECT admin_sessions.user_id, admin_users.role FROM admin_sessions JOIN admin_users ON admin_users.id = admin_sessions.user_id WHERE admin_sessions.token_hash = ? AND admin_sessions.expires_at >= ?",
    )
    .bind(secret_hash(token))
    .bind(unix_after(Duration::ZERO))
    .fetch_optional(database)
    .await
    .ok()??;
    Some(Principal {
        user_id: row.get("user_id"),
        role: Role::parse(row.get::<String, _>("role").as_str())?,
    })
}

async fn write_principal(
    database: &SqlitePool,
    jar: &CookieJar,
    headers: &HeaderMap,
) -> std::result::Result<Principal, axum::response::Response> {
    let Some(principal) = authenticated_principal(database, jar, headers).await else {
        return Err(ApiError::response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        ));
    };
    if !principal.role.can_write() {
        return Err(ApiError::response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "role cannot modify configuration",
        ));
    }
    Ok(principal)
}
async fn audit(
    database: &SqlitePool,
    user_id: Option<&str>,
    source_ip: IpAddr,
    action: &str,
    target: &str,
    success: bool,
) {
    if sqlx::query("INSERT INTO admin_audit_log (created_at, user_id, source_ip, action, target, success) VALUES (?, ?, ?, ?, ?, ?)").bind(unix_after(Duration::ZERO)).bind(user_id).bind(source_ip.to_string()).bind(action).bind(target).bind(success as i64).execute(database).await.is_ok() {
        let _ = sqlx::query(
            "DELETE FROM admin_audit_log WHERE id <= COALESCE((SELECT id FROM admin_audit_log ORDER BY id DESC LIMIT 1 OFFSET ?), -1)",
        )
        .bind(MAX_AUDIT_ENTRIES)
        .execute(database)
        .await;
    }
}
async fn allow_attempt(state: &AdminState, ip: IpAddr) -> bool {
    let now = Instant::now();
    let mut attempts = state.attempts.lock().await;
    if now.duration_since(attempts.last_pruned) >= Duration::from_secs(60) {
        attempts.buckets.retain(|_, bucket| now < bucket.expires_at);
        attempts.last_pruned = now;
    }
    if attempts.buckets.len() >= MAX_AUTH_ATTEMPT_BUCKETS && !attempts.buckets.contains_key(&ip) {
        return false;
    }
    attempts
        .buckets
        .get(&ip)
        .is_none_or(|bucket| now >= bucket.retry_after)
}
async fn fail_attempt(state: &AdminState, ip: IpAddr) {
    let mut attempts = state.attempts.lock().await;
    let bucket = attempts.buckets.entry(ip).or_insert(AttemptBucket {
        failures: 0,
        retry_after: Instant::now(),
        expires_at: Instant::now() + Duration::from_secs(15 * 60),
    });
    bucket.failures = bucket.failures.saturating_add(1);
    let delay = Duration::from_secs(2u64.saturating_pow(bucket.failures.min(8) as u32));
    bucket.retry_after = Instant::now() + delay;
    bucket.expires_at = Instant::now() + Duration::from_secs(15 * 60);
}
async fn clear_attempt(state: &AdminState, ip: IpAddr) {
    state.attempts.lock().await.buckets.remove(&ip);
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Extension, body::Body};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn temporary_directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("webserver-admin-test-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn first_login_requires_the_one_time_setup_code() {
        let directory = temporary_directory();
        tokio::fs::create_dir_all(directory.join("sites"))
            .await
            .expect("create sites directory");
        tokio::fs::create_dir_all(directory.join("public"))
            .await
            .expect("create public directory");
        tokio::fs::write(
            directory.join("webserver.toml"),
            "[server]\nbind = \"127.0.0.1:8080\"\n\n[admin]\nhost = \"admin.test\"\n",
        )
        .await
        .expect("write config");
        tokio::fs::write(directory.join("sites/localhost.conf"), "host = \"localhost\"\n[[routes]]\npath_prefix = \"/\"\nkind = \"static\"\nroot = \"../public\"\n").await.expect("write site");
        let config = Config::load(directory.join("webserver.toml")).expect("load config");
        let database_path = directory.join("admin.db");
        bootstrap(
            &database_path,
            "admin",
            "0123456789abcdef",
            "one-time-setup-code",
        )
        .await
        .expect("bootstrap account");
        let state = AdminState {
            config: Arc::new(RwLock::new(config)),
            database: open_database(&database_path).await.expect("open database"),
            attempts: Arc::new(Mutex::new(AttemptStore::default())),
        };
        let app = router(state.clone()).layer(Extension(ConnectInfo(
            "127.0.0.1:12345".parse::<SocketAddr>().expect("test peer"),
        )));

        let rejected = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("host", "admin.test")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"0123456789abcdef"}"#,
                    ))
                    .expect("build request"),
            )
            .await
            .expect("serve rejected login");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let accepted = router(state.clone()).layer(Extension(ConnectInfo("127.0.0.2:12345".parse::<SocketAddr>().expect("test peer")))).oneshot(
            axum::http::Request::builder().method("POST").uri("/api/v1/auth/login").header("host", "admin.test").header("content-type", "application/json")
                .body(Body::from(r#"{"username":"admin","password":"0123456789abcdef","setup_code":"one-time-setup-code"}"#)).expect("build request")
        ).await.expect("serve accepted login");
        assert_eq!(accepted.status(), StatusCode::OK);
        assert!(accepted.headers().contains_key("set-cookie"));
        let session_cookie = accepted
            .headers()
            .get("set-cookie")
            .expect("session cookie")
            .to_str()
            .expect("valid cookie")
            .split(';')
            .next()
            .expect("cookie name and value")
            .to_owned();
        let body = accepted
            .into_body()
            .collect()
            .await
            .expect("read response")
            .to_bytes();
        assert!(
            std::str::from_utf8(&body)
                .expect("json response")
                .contains("password_change_required")
        );

        let routes = router(state)
            .layer(Extension(ConnectInfo(
                "127.0.0.2:12345".parse::<SocketAddr>().expect("test peer"),
            )))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/sites/localhost/routes")
                    .header("host", "admin.test")
                    .header("cookie", session_cookie)
                    .body(Body::empty())
                    .expect("build route request"),
            )
            .await
            .expect("serve route list");
        assert_eq!(routes.status(), StatusCode::OK);

        // SQLite can hold an advisory file handle briefly on Windows after its
        // last async statement. The unique temporary directory is left for the
        // OS temporary-file cleanup instead of making this security test flaky.
    }

    #[tokio::test]
    async fn openapi_contract_keeps_the_v1_management_surface() {
        let Json(contract) = openapi().await;
        assert_eq!(contract["openapi"], "3.1.0");
        assert_eq!(contract["info"]["version"], "v1");
        for path in [
            "/api/v1/health",
            "/api/v1/auth/login",
            "/api/v1/sites",
            "/api/v1/upstreams",
            "/api/v1/certificates",
            "/api/v1/logs",
            "/api/v1/metrics",
            "/api/v1/observability",
        ] {
            assert!(contract["paths"].get(path).is_some(), "missing {path}");
        }
        assert_eq!(API_VERSION, 1);
    }
}
