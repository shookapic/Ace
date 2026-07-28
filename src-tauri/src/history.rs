use std::fs;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use tauri::{command, Manager};

const KEYRING_SERVICE: &str = "com.foxolf.ace";
const KEYRING_ENTRY: &str = "history-key";

/// Fetch the AES key from the OS keychain, generating and storing one the first
/// time. Returns the raw 32 bytes.
fn get_or_create_key() -> Result<[u8; 32], String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENTRY)
        .map_err(|e| format!("keychain unavailable: {e}"))?;

    match entry.get_password() {
        Ok(encoded) => {
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|e| format!("stored key corrupt: {e}"))?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| "stored key has wrong length".to_string())?;
            Ok(arr)
        }
        Err(_) => {
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            entry
                .set_password(&STANDARD.encode(key))
                .map_err(|e| format!("failed to store key: {e}"))?;
            Ok(key)
        }
    }
}

fn history_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create data dir: {e}"))?;
    Ok(dir.join("history.enc"))
}

/// Decrypt and return the saved conversations JSON, or `"[]"` if there's nothing
/// saved yet or anything goes wrong (fail safe — never crash on bad state).
#[command]
pub fn load_conversations(app: tauri::AppHandle) -> String {
    (|| -> Result<String, String> {
        let path = history_path(&app)?;
        if !path.exists() {
            return Ok("[]".to_string());
        }
        let blob = fs::read(&path).map_err(|e| format!("read failed: {e}"))?;
        if blob.len() < 12 {
            return Ok("[]".to_string());
        }
        let key = get_or_create_key()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let (nonce, ciphertext) = blob.split_at(12);
        let plaintext = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| "decrypt failed".to_string())?;
        String::from_utf8(plaintext).map_err(|e| format!("utf8 failed: {e}"))
    })()
    .unwrap_or_else(|e| {
        eprintln!("load_conversations: {e}");
        "[]".to_string()
    })
}

/// Encrypt and persist the conversations JSON to disk.
#[command]
pub fn save_conversations(app: tauri::AppHandle, json: String) -> Result<(), String> {
    let path = history_path(&app)?;
    let key = get_or_create_key()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), json.as_bytes())
        .map_err(|_| "encrypt failed".to_string())?;

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    fs::write(&path, out).map_err(|e| format!("write failed: {e}"))
}
