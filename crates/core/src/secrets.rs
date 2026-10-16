//! Managed secrets: one passphrase-encrypted age file (`secrets.age`) in the
//! configuration folder holding named entries (SMB logins or small texts).
//! Safe to commit. At deploy time entries are written to
//! /var/lib/nixos-manager/secrets/<name> as root-only files, which modules
//! reference by path (e.g. `credentials=/var/lib/nixos-manager/secrets/nas`).
//!
//! The file is a standard age passphrase file: `age -d secrets.age` recovers
//! the JSON payload without this app.

use crate::types::{SecretEntry, SecretKind, SecretsDeployInfo, SecretsStatus};
use crate::{CoreError, Result};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const FILE_NAME: &str = "secrets.age";
pub const DEPLOY_DIR: &str = "/var/lib/nixos-manager/secrets";
const LOCK_AFTER: Duration = Duration::from_secs(15 * 60);

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct Payload {
    version: u32,
    entries: HashMap<String, SecretEntry>,
}

pub fn file_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

pub fn file_hash(dir: &Path) -> Option<String> {
    std::fs::read(file_path(dir)).ok().map(|b| hex::encode(Sha256::digest(&b)))
}

fn deployed_hash() -> Option<String> {
    std::fs::read_to_string(format!("{DEPLOY_DIR}/.deployed-hash")).ok().map(|s| s.trim().to_string())
}

fn encrypt(payload: &Payload, password: &str) -> Result<Vec<u8>> {
    let json = serde_json::to_vec_pretty(payload)?;
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(password.to_string()));
    let mut out = Vec::new();
    let mut w = encryptor.wrap_output(&mut out).map_err(|e| CoreError::Other(format!("encrypt: {e}")))?;
    w.write_all(&json)?;
    w.finish()?;
    Ok(out)
}

fn decrypt(bytes: &[u8], password: &str) -> Result<Payload> {
    let decryptor = age::Decryptor::new_buffered(bytes).map_err(|e| CoreError::Invalid(format!("not a valid age file: {e}")))?;
    let identity = age::scrypt::Identity::new(SecretString::from(password.to_string()));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| match e {
            age::DecryptError::DecryptionFailed => CoreError::Forbidden("wrong secrets password".into()),
            other => CoreError::Invalid(format!("cannot decrypt: {other}")),
        })?;
    let mut json = Vec::new();
    reader.read_to_end(&mut json).map_err(|_| CoreError::Forbidden("wrong secrets password".into()))?;
    Ok(serde_json::from_slice(&json)?)
}

fn write_file(dir: &Path, payload: &Payload, password: &str) -> Result<()> {
    let bytes = encrypt(payload, password)?;
    let path = file_path(dir);
    let tmp = path.with_extension("age.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

struct Unlocked {
    payload: Payload,
    password: String,
    last_used: Instant,
}

/// In-memory unlocked stores, keyed by config folder.
#[derive(Clone, Default)]
pub struct SecretsVault {
    unlocked: Arc<Mutex<HashMap<String, Unlocked>>>,
}

impl SecretsVault {
    fn key(dir: &Path) -> String {
        dir.to_string_lossy().to_string()
    }

    pub async fn status(&self, dir: &Path) -> SecretsStatus {
        let exists = file_path(dir).is_file();
        let mut map = self.unlocked.lock().await;
        let key = Self::key(dir);
        if let Some(u) = map.get(&key) {
            if u.last_used.elapsed() > LOCK_AFTER {
                map.remove(&key);
            }
        }
        let entries = map.get(&key).map(|u| {
            let mut v: Vec<SecretEntry> = u.payload.entries.values().cloned().map(redact).collect();
            v.sort_by(|a, b| a.name.cmp(&b.name));
            v
        });
        SecretsStatus {
            exists,
            unlocked: entries.is_some(),
            entries: entries.unwrap_or_default(),
            deploy: deploy_info(dir),
        }
    }

    pub async fn create(&self, dir: &Path, password: &str) -> Result<SecretsStatus> {
        if file_path(dir).exists() {
            return Err(CoreError::Conflict("secrets.age already exists".into()));
        }
        if password.len() < 4 {
            return Err(CoreError::Invalid("choose a longer password".into()));
        }
        let payload = Payload { version: 1, entries: HashMap::new() };
        write_file(dir, &payload, password)?;
        self.unlocked.lock().await.insert(Self::key(dir), Unlocked { payload, password: password.into(), last_used: Instant::now() });
        Ok(self.status(dir).await)
    }

    pub async fn unlock(&self, dir: &Path, password: &str) -> Result<SecretsStatus> {
        let bytes = std::fs::read(file_path(dir)).map_err(|_| CoreError::NotFound("no secrets.age in this folder".into()))?;
        let payload = decrypt(&bytes, password)?;
        self.unlocked.lock().await.insert(Self::key(dir), Unlocked { payload, password: password.into(), last_used: Instant::now() });
        Ok(self.status(dir).await)
    }

    pub async fn lock(&self, dir: &Path) -> SecretsStatus {
        self.unlocked.lock().await.remove(&Self::key(dir));
        self.status(dir).await
    }

    async fn with_unlocked<T>(&self, dir: &Path, f: impl FnOnce(&mut Unlocked) -> Result<T>) -> Result<T> {
        let mut map = self.unlocked.lock().await;
        let u = map.get_mut(&Self::key(dir)).ok_or_else(|| CoreError::Forbidden("secrets are locked; unlock them first".into()))?;
        if u.last_used.elapsed() > LOCK_AFTER {
            map.remove(&Self::key(dir));
            return Err(CoreError::Forbidden("secrets were locked after inactivity; unlock them again".into()));
        }
        u.last_used = Instant::now();
        f(u)
    }

    pub async fn get(&self, dir: &Path, name: &str) -> Result<SecretEntry> {
        self.with_unlocked(dir, |u| u.payload.entries.get(name).cloned().ok_or_else(|| CoreError::NotFound(format!("secret {name}")))).await
    }

    pub async fn set(&self, dir: &Path, entry: SecretEntry) -> Result<SecretsStatus> {
        validate_name(&entry.name)?;
        let dir_owned = dir.to_path_buf();
        self.with_unlocked(dir, |u| {
            u.payload.entries.insert(entry.name.clone(), entry);
            write_file(&dir_owned, &u.payload, &u.password)
        })
        .await?;
        Ok(self.status(dir).await)
    }

    pub async fn remove(&self, dir: &Path, name: &str) -> Result<SecretsStatus> {
        let dir_owned = dir.to_path_buf();
        self.with_unlocked(dir, |u| {
            if u.payload.entries.remove(name).is_none() {
                return Err(CoreError::NotFound(format!("secret {name}")));
            }
            write_file(&dir_owned, &u.payload, &u.password)
        })
        .await?;
        Ok(self.status(dir).await)
    }

    pub async fn change_password(&self, dir: &Path, new_password: &str) -> Result<SecretsStatus> {
        if new_password.len() < 4 {
            return Err(CoreError::Invalid("choose a longer password".into()));
        }
        let dir_owned = dir.to_path_buf();
        self.with_unlocked(dir, |u| {
            u.password = new_password.to_string();
            write_file(&dir_owned, &u.payload, &u.password)
        })
        .await?;
        Ok(self.status(dir).await)
    }
}

fn redact(mut e: SecretEntry) -> SecretEntry {
    if !e.password.is_empty() {
        e.password = "••••••••".into();
    }
    if !e.content.is_empty() {
        e.content = format!("({} characters)", e.content.chars().count());
    }
    e
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(CoreError::Invalid("secret names use letters, digits, - and _".into()));
    }
    Ok(())
}

pub fn deploy_info(dir: &Path) -> SecretsDeployInfo {
    let present = file_path(dir).is_file();
    let hash = file_hash(dir);
    let deployed = deployed_hash();
    SecretsDeployInfo {
        present,
        deployed_up_to_date: present && hash.is_some() && hash == deployed,
        deploy_dir: DEPLOY_DIR.into(),
    }
}

/// The on-disk file content for an entry (what a module reads at runtime).
pub fn render_entry(e: &SecretEntry) -> String {
    match e.kind {
        SecretKind::Smb => {
            let mut s = format!("username={}\npassword={}\n", e.username, e.password);
            if !e.domain.is_empty() {
                s.push_str(&format!("domain={}\n", e.domain));
            }
            s
        }
        SecretKind::Text => e.content.clone(),
    }
}

/// Decrypt with `password` and write every entry into a fresh private temp
/// directory. Returns (temp dir, file hash) for the root deploy step to
/// install from. The caller removes the directory afterwards.
pub fn stage_for_deploy(dir: &Path, password: &str) -> Result<(PathBuf, String)> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let bytes = std::fs::read(file_path(dir)).map_err(|_| CoreError::NotFound("no secrets.age in this folder".into()))?;
    let payload = decrypt(&bytes, password)?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let tmp = std::env::temp_dir().join(format!("nixos-manager-secrets-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700))?;
    for (name, e) in &payload.entries {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(tmp.join(name))?;
        f.write_all(render_entry(e).as_bytes())?;
    }
    Ok((tmp, hash))
}

/// Shell snippet (run as root) that installs staged secrets and records the hash.
pub fn install_script(staged: &Path, hash: &str) -> String {
    let src = staged.to_string_lossy();
    format!(
        r#"echo "[nixos-manager] installing managed secrets into {DEPLOY_DIR}"
install -d -m 755 -o root -g root /var/lib/nixos-manager
install -d -m 755 -o root -g root {DEPLOY_DIR}
# remove entries that no longer exist, then install the current ones
for f in {DEPLOY_DIR}/*; do
  [ -e "$f" ] || continue
  n=$(basename "$f")
  [ -e '{src}'/"$n" ] || rm -f "$f"
done
for f in '{src}'/*; do
  [ -e "$f" ] || continue
  install -m 600 -o root -g root "$f" {DEPLOY_DIR}/"$(basename "$f")"
done
printf '%s\n' '{hash}' > {DEPLOY_DIR}/.deployed-hash
chmod 644 {DEPLOY_DIR}/.deployed-hash
rm -rf '{src}'
echo "[nixos-manager] secrets installed"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_password() {
        let mut p = Payload { version: 1, entries: HashMap::new() };
        p.entries.insert("nas".into(), SecretEntry { name: "nas".into(), kind: SecretKind::Smb, username: "vincent".into(), password: "s3cret".into(), domain: String::new(), content: String::new() });
        let bytes = encrypt(&p, "hunter2").unwrap();
        assert!(bytes.starts_with(b"age-encryption.org/v1"));
        let back = decrypt(&bytes, "hunter2").unwrap();
        assert_eq!(back.entries["nas"].password, "s3cret");
        assert!(matches!(decrypt(&bytes, "nope"), Err(CoreError::Forbidden(_))));
        assert_eq!(render_entry(&back.entries["nas"]), "username=vincent\npassword=s3cret\n");
    }

    #[tokio::test]
    async fn vault_flow() {
        let d = tempdir::TempDir::new("secrets").unwrap();
        let v = SecretsVault::default();
        let st = v.status(d.path()).await;
        assert!(!st.exists && !st.unlocked);
        v.create(d.path(), "pass1234").await.unwrap();
        v.set(d.path(), SecretEntry { name: "nas".into(), kind: SecretKind::Smb, username: "u".into(), password: "p".into(), domain: "".into(), content: "".into() }).await.unwrap();
        let st = v.lock(d.path()).await;
        assert!(st.exists && !st.unlocked);
        assert!(v.get(d.path(), "nas").await.is_err());
        assert!(v.unlock(d.path(), "wrong").await.is_err());
        let st = v.unlock(d.path(), "pass1234").await.unwrap();
        assert_eq!(st.entries.len(), 1);
        assert_eq!(st.entries[0].password, "••••••••");
        assert_eq!(v.get(d.path(), "nas").await.unwrap().password, "p");
        v.change_password(d.path(), "newpass").await.unwrap();
        v.lock(d.path()).await;
        assert!(v.unlock(d.path(), "newpass").await.is_ok());
        let (tmp, hash) = stage_for_deploy(d.path(), "newpass").unwrap();
        assert_eq!(std::fs::read_to_string(tmp.join("nas")).unwrap(), "username=u\npassword=p\n");
        assert_eq!(hash.len(), 64);
        let script = install_script(&tmp, &hash);
        assert!(script.contains("install -m 600 -o root -g root"));
        std::fs::remove_dir_all(tmp).unwrap();
    }
}
