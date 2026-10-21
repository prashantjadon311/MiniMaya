use crate::error::{AppError, ErrorCode, Result};
use crate::secrets::SecretString;
use crate::types::{Digest, Id, UtcMs};
use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use rand::RngCore;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct LaunchLink {
    pub url: SecretString,
    pub expires_at: UtcMs,
}

#[derive(Debug, Clone)]
pub struct SessionGrant {
    pub session_id: Id,
    pub session_token: String,
    pub csrf_token: String,
    pub expires_at: UtcMs,
}

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub id: Id,
    pub owner_uid: u32,
    pub expires_at: UtcMs,
    pub csrf_hash: Digest,
}

struct BootstrapToken {
    token_hash: Digest,
    expires_at: UtcMs,
    used: bool,
}

struct ActiveSession {
    session_id: Id,
    session_token_hash: Digest,
    csrf_hash: Digest,
    owner_uid: u32,
    created_at: UtcMs,
    last_active_at: UtcMs,
    expires_at: UtcMs, // absolute max (8h)
}

#[derive(Clone)]
pub struct AuthService {
    bootstraps: Arc<RwLock<Vec<BootstrapToken>>>,
    sessions: Arc<RwLock<HashMap<Id, ActiveSession>>>,
    failed_bootstraps: Arc<RwLock<Vec<UtcMs>>>,
    mutation_times: Arc<RwLock<HashMap<Id, VecDeque<UtcMs>>>>,
    stream_slots: Arc<Semaphore>,
    port: u16,
}

impl AuthService {
    pub fn new(port: u16) -> Self {
        Self {
            bootstraps: Arc::new(RwLock::new(Vec::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            failed_bootstraps: Arc::new(RwLock::new(Vec::new())),
            mutation_times: Arc::new(RwLock::new(HashMap::new())),
            stream_slots: Arc::new(Semaphore::new(4)),
            port,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    pub async fn issue_launch_link(&self, port: u16) -> Result<LaunchLink> {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = B64URL.encode(bytes);

        let token_hash = Digest::of(token.as_bytes());
        let now = chrono::Utc::now().timestamp_millis();
        let expires_at = now + 120_000; // 120 seconds TTL

        let mut lock = self.bootstraps.write().await;
        // Purge expired
        lock.retain(|b| b.expires_at > now && !b.used);
        lock.push(BootstrapToken {
            token_hash,
            expires_at,
            used: false,
        });

        let url_str = format!("http://127.0.0.1:{}/#bootstrap={}", port, token);
        Ok(LaunchLink {
            url: SecretString::new(url_str),
            expires_at,
        })
    }

    pub async fn exchange(&self, token: SecretString, origin: &str) -> Result<SessionGrant> {
        let now = chrono::Utc::now().timestamp_millis();

        // Rate limiting: max 5 failed / min
        {
            let mut failed = self.failed_bootstraps.write().await;
            failed.retain(|t| now - *t < 60_000);
            if failed.len() >= 5 {
                return Err(AppError::new(
                    ErrorCode::RateLimited,
                    "Too many failed bootstrap attempts. Please wait.",
                ));
            }
        }

        // Verify origin matches exact loopback
        let expected_origin = format!("http://127.0.0.1:{}", self.port);
        if origin != expected_origin {
            return Err(AppError::new(
                ErrorCode::OriginDenied,
                format!("Invalid origin: {origin}, expected {expected_origin}"),
            ));
        }

        let input_hash = Digest::of(token.expose_str().as_bytes());

        let mut bootstraps = self.bootstraps.write().await;
        let mut found = false;
        for b in bootstraps.iter_mut() {
            if b.expires_at > now && !b.used && b.token_hash.0.ct_eq(&input_hash.0).into() {
                b.used = true;
                found = true;
                break;
            }
        }

        if !found {
            let mut failed = self.failed_bootstraps.write().await;
            failed.push(now);
            return Err(AppError::new(
                ErrorCode::BootstrapInvalid,
                "Invalid or expired bootstrap token",
            ));
        }

        // Create new session
        let mut session_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut session_bytes);
        let session_token = B64URL.encode(session_bytes);
        let session_token_hash = Digest::of(session_token.as_bytes());

        let mut csrf_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut csrf_bytes);
        let csrf_token = B64URL.encode(csrf_bytes);
        let csrf_hash = Digest::of(csrf_token.as_bytes());

        let session_id = Uuid::new_v4();
        let uid = rustix::process::getuid().as_raw();
        let expires_at = now + 8 * 3600 * 1000; // 8 hours absolute max

        let mut sessions = self.sessions.write().await;
        // Purge expired sessions
        sessions.retain(|_, s| s.expires_at > now && now - s.last_active_at < 30 * 60 * 1000);

        // Enforce max 8 sessions limit
        if sessions.len() >= 8 {
            // Remove oldest
            if let Some(oldest_id) = sessions
                .iter()
                .min_by_key(|(_, s)| s.created_at)
                .map(|(id, _)| *id)
            {
                sessions.remove(&oldest_id);
            }
        }

        sessions.insert(
            session_id,
            ActiveSession {
                session_id,
                session_token_hash,
                csrf_hash,
                owner_uid: uid,
                created_at: now,
                last_active_at: now,
                expires_at,
            },
        );

        Ok(SessionGrant {
            session_id,
            session_token,
            csrf_token,
            expires_at,
        })
    }

    pub async fn authenticate_token(&self, token_str: &str) -> Result<SessionContext> {
        let input_hash = Digest::of(token_str.as_bytes());
        let now = chrono::Utc::now().timestamp_millis();

        let mut sessions = self.sessions.write().await;
        for s in sessions.values_mut() {
            if s.session_token_hash.0.ct_eq(&input_hash.0).into() {
                // Check absolute expiry (8 hours)
                if now > s.expires_at {
                    return Err(AppError::new(ErrorCode::AuthExpired, "Session expired"));
                }
                // Check idle expiry (30 minutes)
                if now - s.last_active_at > 30 * 60 * 1000 {
                    return Err(AppError::new(
                        ErrorCode::AuthExpired,
                        "Session expired due to inactivity",
                    ));
                }
                s.last_active_at = now;

                return Ok(SessionContext {
                    id: s.session_id,
                    owner_uid: s.owner_uid,
                    expires_at: s.expires_at,
                    csrf_hash: s.csrf_hash,
                });
            }
        }

        Err(AppError::new(
            ErrorCode::AuthRequired,
            "Invalid bearer authentication token",
        ))
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<SessionContext> {
        let auth_hdr = headers
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| {
                AppError::new(ErrorCode::AuthRequired, "Missing Authorization header")
            })?;

        let token = auth_hdr
            .strip_prefix("Bearer ")
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::AuthRequired,
                    "Invalid Authorization header format, expected Bearer <token>",
                )
            })?
            .trim();

        self.authenticate_token(token).await
    }

    pub fn csrf(&self, session: &SessionContext, headers: &HeaderMap) -> Result<()> {
        let csrf_hdr = headers
            .get("X-CSRF-Token")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AppError::new(ErrorCode::CsrfDenied, "Missing X-CSRF-Token header"))?;

        let input_hash = Digest::of(csrf_hdr.trim().as_bytes());
        if !bool::from(session.csrf_hash.0.ct_eq(&input_hash.0)) {
            return Err(AppError::new(ErrorCode::CsrfDenied, "CSRF token mismatch"));
        }
        Ok(())
    }

    pub async fn revoke(&self, id: Id) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(&id);
        self.mutation_times.write().await.remove(&id);
        Ok(())
    }

    pub async fn admit_mutation(&self, id: Id) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut all = self.mutation_times.write().await;
        let times = all.entry(id).or_default();
        while times.front().is_some_and(|at| now - *at >= 60_000) {
            times.pop_front();
        }
        if times.len() >= 120 {
            return Err(AppError::new(
                ErrorCode::RateLimited,
                "Session mutation rate limit exceeded",
            ));
        }
        times.push_back(now);
        Ok(())
    }

    pub fn acquire_stream(&self) -> Result<OwnedSemaphorePermit> {
        self.stream_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::new(ErrorCode::RateLimited, "Too many live event streams"))
    }
}
