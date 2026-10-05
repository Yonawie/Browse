use base64::Engine;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{new_id, now_ms, MemoryStore, Result};

const DEFAULT_VAULT_KEY: &str = "browse.local.zero_knowledge.device_seed.v1";

/// A saved login credential record (without revealing plaintext secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCredential {
    pub id: String,
    pub profile_id: String,
    pub origin: String,
    pub domain: String,
    pub username: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// Input payload to persist a new login credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCredential {
    pub origin: String,
    pub username: String,
    pub secret: String,
}

/// Derive a cryptographic 256-bit key using BLAKE3 KDF.
pub fn derive_vault_key(master: &str) -> [u8; 32] {
    blake3::derive_key("browse.vault.credentials.v1", master.as_bytes())
}

/// Generate a 16-byte random nonce for stream cipher encryption.
pub fn generate_nonce() -> [u8; 16] {
    let u = uuid::Uuid::new_v4();
    *u.as_bytes()
}

fn generate_keystream(key: &[u8; 32], nonce: &[u8; 16], len: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(len);
    let mut counter = 0u64;
    while output.len() < len {
        let mut hasher = blake3::Hasher::new_keyed(key);
        hasher.update(nonce);
        hasher.update(&counter.to_le_bytes());
        let block = hasher.finalize();
        let needed = len - output.len();
        let take = needed.min(32);
        output.extend_from_slice(&block.as_bytes()[..take]);
        counter += 1;
    }
    output
}

/// Encrypts plaintext secret into a base64 ciphertext using BLAKE3 keystream XOR cipher.
pub fn encrypt_secret(key: &[u8; 32], nonce: &[u8; 16], plaintext: &str) -> String {
    let bytes = plaintext.as_bytes();
    let keystream = generate_keystream(key, nonce, bytes.len());
    let cipher: Vec<u8> = bytes.iter().zip(keystream.iter()).map(|(b, k)| b ^ k).collect();
    base64::engine::general_purpose::STANDARD.encode(cipher)
}

/// Decrypts a base64 ciphertext using BLAKE3 keystream XOR cipher.
pub fn decrypt_secret(key: &[u8; 32], nonce: &[u8; 16], ciphertext_b64: &str) -> std::result::Result<String, String> {
    let cipher = base64::engine::general_purpose::STANDARD
        .decode(ciphertext_b64)
        .map_err(|e| format!("Base64 decode error: {e}"))?;
    let keystream = generate_keystream(key, nonce, cipher.len());
    let plain: Vec<u8> = cipher.iter().zip(keystream.iter()).map(|(c, k)| c ^ k).collect();
    String::from_utf8(plain).map_err(|e| format!("UTF-8 decode error: {e}"))
}

pub fn origin_to_domain(origin: &str) -> String {
    let lower = origin.to_lowercase();
    if let Some(pos) = lower.find("://") {
        let rest = &lower[pos + 3..];
        let host = rest.split(&['/', ':', '?', '#'][..]).next().unwrap_or("");
        host.trim_start_matches("www.").to_string()
    } else {
        lower.trim_start_matches("www.").to_string()
    }
}

impl MemoryStore {
    /// Ensure the credentials schema table exists.
    pub fn ensure_vault_schema(&self) -> Result<()> {
        self.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS credentials (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                origin TEXT NOT NULL,
                domain TEXT NOT NULL,
                username TEXT NOT NULL,
                encrypted_secret TEXT NOT NULL,
                nonce TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_used_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_credentials_domain ON credentials(domain);
            CREATE INDEX IF NOT EXISTS idx_credentials_profile ON credentials(profile_id);",
        )?;
        Ok(())
    }

    /// Save an encrypted login credential into the local vault.
    pub fn save_credential(
        &self,
        profile_id: &str,
        cred: &NewCredential,
        master_key: Option<&str>,
    ) -> Result<String> {
        self.ensure_profile(profile_id, "user", profile_id)?;
        self.ensure_vault_schema()?;
        let id = new_id();
        let now = now_ms();
        let domain = origin_to_domain(&cred.origin);

        let key = derive_vault_key(master_key.unwrap_or(DEFAULT_VAULT_KEY));
        let nonce = generate_nonce();
        let encrypted_secret = encrypt_secret(&key, &nonce, &cred.secret);
        let nonce_hex = hex::encode(nonce);

        self.conn().execute(
            "INSERT INTO credentials (id, profile_id, origin, domain, username, encrypted_secret, nonce, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            params![
                id,
                profile_id,
                cred.origin,
                domain,
                cred.username,
                encrypted_secret,
                nonce_hex,
                now
            ],
        )?;

        Ok(id)
    }

    /// List credentials metadata without revealing decrypted passwords.
    pub fn list_credentials(
        &self,
        profile_id: &str,
        domain_filter: Option<&str>,
    ) -> Result<Vec<VaultCredential>> {
        self.ensure_vault_schema()?;
        let conn = self.conn();

        let mut list = Vec::new();
        if let Some(domain) = domain_filter {
            let clean_dom = domain.trim_start_matches("www.").to_lowercase();
            let mut stmt = conn.prepare(
                "SELECT id, profile_id, origin, domain, username, created_at, last_used_at
                 FROM credentials
                 WHERE profile_id = ?1 AND domain = ?2
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![profile_id, clean_dom], |row| {
                Ok(VaultCredential {
                    id: row.get(0)?,
                    profile_id: row.get(1)?,
                    origin: row.get(2)?,
                    domain: row.get(3)?,
                    username: row.get(4)?,
                    created_at: row.get(5)?,
                    last_used_at: row.get(6)?,
                })
            })?;
            for r in rows {
                list.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, profile_id, origin, domain, username, created_at, last_used_at
                 FROM credentials
                 WHERE profile_id = ?1
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![profile_id], |row| {
                Ok(VaultCredential {
                    id: row.get(0)?,
                    profile_id: row.get(1)?,
                    origin: row.get(2)?,
                    domain: row.get(3)?,
                    username: row.get(4)?,
                    created_at: row.get(5)?,
                    last_used_at: row.get(6)?,
                })
            })?;
            for r in rows {
                list.push(r?);
            }
        }

        Ok(list)
    }

    /// Retrieve and decrypt a credential password.
    pub fn get_credential_secret(
        &self,
        id: &str,
        master_key: Option<&str>,
    ) -> Result<Option<String>> {
        self.ensure_vault_schema()?;
        let conn = self.conn();
        let query_res: rusqlite::Result<(String, String)> = conn.query_row(
            "SELECT encrypted_secret, nonce FROM credentials WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );

        match query_res {
            Ok((encrypted_secret, nonce_hex)) => {
                let nonce_bytes = hex::decode(&nonce_hex).unwrap_or_default();
                if nonce_bytes.len() != 16 {
                    return Ok(None);
                }
                let mut nonce = [0u8; 16];
                nonce.copy_from_slice(&nonce_bytes);

                let key = derive_vault_key(master_key.unwrap_or(DEFAULT_VAULT_KEY));
                match decrypt_secret(&key, &nonce, &encrypted_secret) {
                    Ok(plain) => {
                        let _ = conn.execute(
                            "UPDATE credentials SET last_used_at = ?1 WHERE id = ?2",
                            params![now_ms(), id],
                        );
                        Ok(Some(plain))
                    }
                    Err(_) => Ok(None),
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a credential from the vault.
    pub fn delete_credential(&self, id: &str) -> Result<bool> {
        self.ensure_vault_schema()?;
        let count = self.conn().execute("DELETE FROM credentials WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }
}

// Minimal hex helper
mod hex {
    pub fn encode(data: [u8; 16]) -> String {
        let mut s = String::with_capacity(32);
        for b in data {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        if !s.len().is_multiple_of(2) {
            return None;
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_and_decrypt_reproduces_secret() {
        let key = derive_vault_key("my_master_password_123");
        let nonce = generate_nonce();
        let secret = "SuperSecretP@ssw0rd!#2026";

        let encrypted = encrypt_secret(&key, &nonce, secret);
        assert_ne!(encrypted, secret);

        let decrypted = decrypt_secret(&key, &nonce, &encrypted).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn saves_lists_and_deletes_credentials_in_store() {
        let store = MemoryStore::open_in_memory().unwrap();

        let new_c = NewCredential {
            origin: "https://github.com/login".to_string(),
            username: "octocat".to_string(),
            secret: "ghp_tok123456789".to_string(),
        };

        let id = store.save_credential("default", &new_c, None).unwrap();
        let list = store.list_credentials("default", Some("github.com")).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].domain, "github.com");
        assert_eq!(list[0].username, "octocat");

        let plain = store.get_credential_secret(&id, None).unwrap();
        assert_eq!(plain, Some("ghp_tok123456789".to_string()));

        let deleted = store.delete_credential(&id).unwrap();
        assert!(deleted);
        assert!(store.list_credentials("default", None).unwrap().is_empty());
    }
}
