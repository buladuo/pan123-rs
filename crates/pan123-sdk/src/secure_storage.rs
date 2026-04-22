use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rand::RngCore;
use std::fs;
use std::path::PathBuf;

use crate::error::{Pan123Error, Result};

const SERVICE_NAME: &str = "pan123-cli";
const TOKEN_USERNAME: &str = "default";
const NONCE_SIZE: usize = 12;

pub enum StorageBackend {
    Keyring,
    EncryptedFile,
    PlaintextFile,
}

impl StorageBackend {
    pub fn best_available() -> Self {
        if keyring::Entry::new(SERVICE_NAME, TOKEN_USERNAME).is_ok() {
            Self::Keyring
        } else {
            Self::EncryptedFile
        }
    }
}

pub struct SecureStorage {
    backend: StorageBackend,
    file_path: Option<PathBuf>,
}

impl SecureStorage {
    pub fn new(backend: StorageBackend, file_path: Option<PathBuf>) -> Self {
        Self { backend, file_path }
    }

    pub fn auto(file_path: PathBuf) -> Self {
        Self::new(StorageBackend::best_available(), Some(file_path))
    }

    pub fn save_token(&self, token: &str) -> Result<()> {
        match self.backend {
            StorageBackend::Keyring => self.save_to_keyring(token),
            StorageBackend::EncryptedFile => self.save_encrypted(token),
            StorageBackend::PlaintextFile => self.save_plaintext(token),
        }
    }

    pub fn load_token(&self) -> Option<String> {
        match self.backend {
            StorageBackend::Keyring => self.load_from_keyring().ok(),
            StorageBackend::EncryptedFile => self.load_encrypted().ok(),
            StorageBackend::PlaintextFile => self.load_plaintext().ok(),
        }
    }

    pub fn delete_token(&self) -> Result<()> {
        match self.backend {
            StorageBackend::Keyring => self.delete_from_keyring(),
            StorageBackend::EncryptedFile | StorageBackend::PlaintextFile => {
                if let Some(path) = &self.file_path {
                    fs::remove_file(path).ok();
                }
                Ok(())
            }
        }
    }

    fn save_to_keyring(&self, token: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE_NAME, TOKEN_USERNAME)
            .map_err(|e| Pan123Error::Operation(format!("keyring init failed: {e}")))?;
        entry
            .set_password(token)
            .map_err(|e| Pan123Error::Operation(format!("keyring save failed: {e}")))?;
        Ok(())
    }

    fn load_from_keyring(&self) -> Result<String> {
        let entry = keyring::Entry::new(SERVICE_NAME, TOKEN_USERNAME)
            .map_err(|e| Pan123Error::Operation(format!("keyring init failed: {e}")))?;
        entry
            .get_password()
            .map_err(|e| Pan123Error::Operation(format!("keyring load failed: {e}")))
    }

    fn delete_from_keyring(&self) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE_NAME, TOKEN_USERNAME)
            .map_err(|e| Pan123Error::Operation(format!("keyring init failed: {e}")))?;
        entry
            .delete_credential()
            .map_err(|e| Pan123Error::Operation(format!("keyring delete failed: {e}")))?;
        Ok(())
    }

    fn save_encrypted(&self, token: &str) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| Pan123Error::Operation("file path required".into()))?;

        let key = self.derive_machine_key()?;
        let cipher = Aes256Gcm::new(&key.into());

        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, token.as_bytes())
            .map_err(|e| Pan123Error::Operation(format!("encryption failed: {e}")))?;

        let mut payload = nonce_bytes.to_vec();
        payload.extend_from_slice(&ciphertext);
        let encoded = BASE64.encode(&payload);

        fs::write(path, encoded)?;
        Ok(())
    }

    fn load_encrypted(&self) -> Result<String> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| Pan123Error::Operation("file path required".into()))?;

        let encoded = fs::read_to_string(path)?;
        let payload = BASE64
            .decode(encoded.trim())
            .map_err(|e| Pan123Error::Operation(format!("base64 decode failed: {e}")))?;

        if payload.len() < NONCE_SIZE {
            return Err(Pan123Error::Operation("invalid encrypted token".into()));
        }

        let (nonce_bytes, ciphertext) = payload.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let key = self.derive_machine_key()?;
        let cipher = Aes256Gcm::new(&key.into());

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Pan123Error::Operation(format!("decryption failed: {e}")))?;

        String::from_utf8(plaintext)
            .map_err(|e| Pan123Error::Operation(format!("invalid utf8: {e}")))
    }

    fn save_plaintext(&self, token: &str) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| Pan123Error::Operation("file path required".into()))?;
        fs::write(path, token)?;
        Ok(())
    }

    fn load_plaintext(&self) -> Result<String> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| Pan123Error::Operation("file path required".into()))?;
        Ok(fs::read_to_string(path)?.trim().to_string())
    }

    fn derive_machine_key(&self) -> Result<[u8; 32]> {
        use sha2::{Digest, Sha256};

        let machine_id = self.get_machine_id()?;
        let mut hasher = Sha256::new();
        hasher.update(SERVICE_NAME.as_bytes());
        hasher.update(b":");
        hasher.update(machine_id.as_bytes());
        let result = hasher.finalize();

        let mut key = [0u8; 32];
        key.copy_from_slice(&result);
        Ok(key)
    }

    fn get_machine_id(&self) -> Result<String> {
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            let output = Command::new("wmic")
                .args(["csproduct", "get", "UUID"])
                .output()
                .map_err(|e| Pan123Error::Operation(format!("failed to get machine id: {e}")))?;
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines()
                .nth(1)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Pan123Error::Operation("machine id not found".into()))
        }

        #[cfg(target_os = "linux")]
        {
            fs::read_to_string("/etc/machine-id")
                .or_else(|_| fs::read_to_string("/var/lib/dbus/machine-id"))
                .map(|s| s.trim().to_string())
                .map_err(|e| Pan123Error::Operation(format!("failed to get machine id: {e}")))
        }

        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            let output = Command::new("ioreg")
                .args(["-rd1", "-c", "IOPlatformExpertDevice"])
                .output()
                .map_err(|e| Pan123Error::Operation(format!("failed to get machine id: {e}")))?;
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines()
                .find(|line| line.contains("IOPlatformUUID"))
                .and_then(|line| line.split('"').nth(3))
                .map(|s| s.to_string())
                .ok_or_else(|| Pan123Error::Operation("machine id not found".into()))
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            Ok(format!(
                "fallback-{}",
                std::env::var("USER").unwrap_or_else(|_| "unknown".into())
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    #[cfg_attr(
        all(target_os = "windows", not(target_env = "msvc")),
        ignore = "Machine ID not available in CI"
    )]
    fn test_encrypted_roundtrip() {
        let temp_dir = env::temp_dir();
        let token_file = temp_dir.join("test_token_encrypted.txt");
        let _ = fs::remove_file(&token_file);

        let storage = SecureStorage::new(StorageBackend::EncryptedFile, Some(token_file.clone()));
        let original = "test-token-12345";

        // Skip test if machine ID is not available (e.g., in CI)
        if storage.save_token(original).is_err() {
            eprintln!("Skipping test: machine ID not available");
            return;
        }
        let loaded = storage.load_token().unwrap();
        assert_eq!(original, loaded);

        let _ = fs::remove_file(&token_file);
    }

    #[test]
    fn test_plaintext_roundtrip() {
        let temp_dir = env::temp_dir();
        let token_file = temp_dir.join("test_token_plain.txt");
        let _ = fs::remove_file(&token_file);

        let storage = SecureStorage::new(StorageBackend::PlaintextFile, Some(token_file.clone()));
        let original = "test-token-67890";

        storage.save_token(original).unwrap();
        let loaded = storage.load_token().unwrap();
        assert_eq!(original, loaded);

        let _ = fs::remove_file(&token_file);
    }
}
