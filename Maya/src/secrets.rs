use crate::error::{AppError, ErrorCode, Result};
use crate::types::{CredentialStatus, Id, SecretMode};
use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use zeroize::Zeroizing;

#[derive(Clone, Default)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(s: impl Into<String>) -> Self {
        Self(Zeroizing::new(s.into()))
    }

    pub fn expose_str(&self) -> &str {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED SECRET]")
    }
}

pub trait SecretStore: Send + Sync {
    fn put<'a>(
        &'a self,
        id: Id,
        secret: SecretString,
        mode: SecretMode,
    ) -> BoxFuture<'a, Result<CredentialStatus>>;

    fn get<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<SecretString>>;

    fn remove<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<()>>;
}

#[derive(Clone, Default)]
pub struct SessionSecrets {
    entries: Arc<RwLock<HashMap<Id, SecretString>>>,
}

impl SessionSecrets {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl SecretStore for SessionSecrets {
    fn put<'a>(
        &'a self,
        id: Id,
        secret: SecretString,
        _mode: SecretMode,
    ) -> BoxFuture<'a, Result<CredentialStatus>> {
        Box::pin(async move {
            let mut lock = self.entries.write().await;
            lock.insert(id, secret);
            Ok(CredentialStatus::Session)
        })
    }

    fn get<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<SecretString>> {
        Box::pin(async move {
            let lock = self.entries.read().await;
            lock.get(&id)
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Secret not found in session"))
        })
    }

    fn remove<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut lock = self.entries.write().await;
            lock.remove(&id);
            Ok(())
        })
    }
}

#[derive(Clone, Default)]
pub struct KeyringSecrets;

impl KeyringSecrets {
    pub fn new() -> Self {
        Self
    }

    fn entry(id: Id) -> Result<keyring::Entry> {
        keyring::Entry::new("maya-mini", &id.to_string()).map_err(|e| {
            AppError::new(
                ErrorCode::KeyringUnavailable,
                format!("Failed to initialize keyring entry: {e}"),
            )
        })
    }
}

impl SecretStore for KeyringSecrets {
    fn put<'a>(
        &'a self,
        id: Id,
        secret: SecretString,
        _mode: SecretMode,
    ) -> BoxFuture<'a, Result<CredentialStatus>> {
        Box::pin(async move {
            let entry = Self::entry(id)?;
            tokio::task::spawn_blocking(move || {
                entry.set_password(secret.expose_str()).map_err(|e| {
                    AppError::new(
                        ErrorCode::KeyringUnavailable,
                        format!("Keyring set_password failed: {e}"),
                    )
                })?;
                Ok(CredentialStatus::Keyring)
            })
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
        })
    }

    fn get<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<SecretString>> {
        Box::pin(async move {
            let entry = Self::entry(id)?;
            tokio::task::spawn_blocking(move || {
                let pass = entry.get_password().map_err(|e| match e {
                    keyring::Error::NoEntry => {
                        AppError::new(ErrorCode::NotFound, "Secret not found in keyring")
                    }
                    other => AppError::new(
                        ErrorCode::KeyringUnavailable,
                        format!("Keyring get_password failed: {other}"),
                    ),
                })?;
                Ok(SecretString::new(pass))
            })
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
        })
    }

    fn remove<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let entry = Self::entry(id)?;
            tokio::task::spawn_blocking(move || match entry.delete_credential() {
                Ok(_) => Ok(()),
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(AppError::new(
                    ErrorCode::KeyringUnavailable,
                    format!("Keyring delete failed: {e}"),
                )),
            })
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
        })
    }
}

#[derive(Clone)]
pub struct AppSecretStore {
    session: Arc<SessionSecrets>,
    keyring: Arc<KeyringSecrets>,
}

impl AppSecretStore {
    pub fn new() -> Self {
        Self {
            session: Arc::new(SessionSecrets::new()),
            keyring: Arc::new(KeyringSecrets::new()),
        }
    }
}

impl Default for AppSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for AppSecretStore {
    fn put<'a>(
        &'a self,
        id: Id,
        secret: SecretString,
        mode: SecretMode,
    ) -> BoxFuture<'a, Result<CredentialStatus>> {
        Box::pin(async move {
            match mode {
                SecretMode::Session => self.session.put(id, secret, mode).await,
                SecretMode::Keyring => {
                    // Try keyring
                    let res = self.keyring.put(id, secret.clone(), mode).await;
                    match res {
                        Ok(status) => Ok(status),
                        Err(e) if e.code == ErrorCode::KeyringUnavailable => {
                            // If keyring unavailable, return error as mandated by BP §9.4
                            Err(e)
                        }
                        Err(e) => Err(e),
                    }
                }
            }
        })
    }

    fn get<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<SecretString>> {
        Box::pin(async move {
            // Check session first
            if let Ok(sec) = self.session.get(id).await {
                return Ok(sec);
            }
            // Check keyring
            self.keyring.get(id).await
        })
    }

    fn remove<'a>(&'a self, id: Id) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let _ = self.session.remove(id).await;
            let _ = self.keyring.remove(id).await;
            Ok(())
        })
    }
}
