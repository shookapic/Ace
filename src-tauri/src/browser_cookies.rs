//! Silent claude.ai session discovery from browsers other than Firefox.
//!
//! Firefox keeps cookie values in plaintext, so [`crate::chat`] reads it
//! directly. The Chromium family and Safari encrypt (or binary-pack) their
//! jars, so each needs its own extractor:
//!
//! - **Chromium (Chrome / Edge / Brave)** — cookies live in an SQLite `Cookies`
//!   DB with `encrypted_value` blobs. Decryption is per-OS:
//!   - Windows: AES-256-GCM, key wrapped by DPAPI in `Local State` (the `v10`
//!     scheme). The newer `v20` app-bound scheme is *not* decryptable from
//!     another process — those cookies are skipped and the caller falls back to
//!     the embedded login webview.
//!   - macOS: AES-128-CBC, key = PBKDF2(Keychain "…​Safe Storage" password).
//!     First read triggers a Keychain permission prompt.
//!   - Linux: AES-128-CBC with the well-known `peanuts` password (v10). The
//!     libsecret-backed `v11` key is not read here; Firefox covers Linux.
//! - **Safari** (macOS only) — `Cookies.binarycookies`, an unencrypted binary
//!   format. Reading Safari's container needs Full Disk Access granted to Ace.
//!
//! Every extractor yields a [`BrowserJar`]: the claude.ai cookie name/value
//! pairs plus the User-Agent that browser presents (`cf_clearance` is loosely
//! keyed to the UA that solved the Cloudflare challenge, so the replay UA must
//! match the source browser).

/// A single browser's claude.ai cookie jar.
pub struct BrowserJar {
    pub cookies: Vec<(String, String)>,
    pub user_agent: String,
}

fn has_session(cookies: &[(String, String)]) -> bool {
    cookies.iter().any(|(n, _)| n == "sessionKey")
}

/// Tries every supported non-Firefox browser in turn, returning the first jar
/// that carries a claude.ai `sessionKey`.
pub fn find_claude_jar() -> Option<BrowserJar> {
    for b in chromium_browsers() {
        if let Some(cookies) = read_chromium(&b) {
            if has_session(&cookies) {
                return Some(BrowserJar { cookies, user_agent: b.user_agent });
            }
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(cookies) = read_safari() {
        if has_session(&cookies) {
            return Some(BrowserJar { cookies, user_agent: SAFARI_UA.to_string() });
        }
    }

    None
}

// A recent stable Chrome UA. cf_clearance is only loosely UA-keyed, so one
// current desktop Chrome string covers Chrome/Edge/Brave replay.
const CHROMIUM_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[cfg(target_os = "macos")]
const SAFARI_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

// ─── Chromium family ────────────────────────────────────────────────────────

struct ChromiumBrowser {
    /// The "User Data" root that holds `Local State` and the profile dirs.
    user_data: std::path::PathBuf,
    /// Keychain service name for the AES key (macOS only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    safe_storage_service: &'static str,
    user_agent: String,
}

fn chromium_browsers() -> Vec<ChromiumBrowser> {
    let mut out = Vec::new();

    #[cfg(target_os = "windows")]
    let roots: Vec<(std::path::PathBuf, &str)> = {
        let base = dirs::data_local_dir();
        base.into_iter()
            .flat_map(|b| {
                vec![
                    (b.join("Google").join("Chrome").join("User Data"), "Chrome Safe Storage"),
                    (b.join("Microsoft").join("Edge").join("User Data"), "Microsoft Edge Safe Storage"),
                    (
                        b.join("BraveSoftware").join("Brave-Browser").join("User Data"),
                        "Brave Safe Storage",
                    ),
                ]
            })
            .collect()
    };

    #[cfg(target_os = "macos")]
    let roots: Vec<(std::path::PathBuf, &str)> = {
        let base = dirs::data_dir();
        base.into_iter()
            .flat_map(|b| {
                vec![
                    (b.join("Google").join("Chrome"), "Chrome Safe Storage"),
                    (b.join("Microsoft Edge"), "Microsoft Edge Safe Storage"),
                    (b.join("BraveSoftware").join("Brave-Browser"), "Brave Safe Storage"),
                ]
            })
            .collect()
    };

    #[cfg(target_os = "linux")]
    let roots: Vec<(std::path::PathBuf, &str)> = {
        let base = dirs::config_dir();
        base.into_iter()
            .flat_map(|b| {
                vec![
                    (b.join("google-chrome"), "Chrome Safe Storage"),
                    (b.join("microsoft-edge"), "Microsoft Edge Safe Storage"),
                    (b.join("BraveSoftware").join("Brave-Browser"), "Brave Safe Storage"),
                ]
            })
            .collect()
    };

    for (user_data, service) in roots {
        if user_data.exists() {
            out.push(ChromiumBrowser {
                user_data,
                safe_storage_service: service,
                user_agent: CHROMIUM_UA.to_string(),
            });
        }
    }
    out
}

/// Reads and decrypts claude.ai cookies from every profile of one Chromium
/// browser, returning the first profile that carries a session.
fn read_chromium(browser: &ChromiumBrowser) -> Option<Vec<(String, String)>> {
    let key = chromium_key(browser)?;

    for db in chromium_cookie_dbs(&browser.user_data) {
        // Copy out — the browser holds a WAL lock on the live file.
        let tmp = std::env::temp_dir().join("ace_chromium_cookies.sqlite");
        if std::fs::copy(&db, &tmp).is_err() {
            continue;
        }
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT name, value, encrypted_value FROM cookies WHERE host_key LIKE '%claude.ai%'",
        ) else {
            continue;
        };
        let rows: Vec<(String, String, Vec<u8>)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1).unwrap_or_default(),
                    r.get::<_, Vec<u8>>(2).unwrap_or_default(),
                ))
            })
            .and_then(|m| m.collect())
            .unwrap_or_default();

        let mut cookies = Vec::new();
        for (name, plain, enc) in rows {
            if !plain.is_empty() {
                cookies.push((name, plain));
            } else if let Some(v) = decrypt_chromium(&enc, &key) {
                cookies.push((name, v));
            }
            // else: a v20 (app-bound) cookie we can't decrypt — skip it.
        }

        if has_session(&cookies) {
            return Some(cookies);
        }
    }
    None
}

/// Candidate `Cookies` DB paths across a browser's profiles (Default, Profile N).
fn chromium_cookie_dbs(user_data: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dbs = Vec::new();
    let Ok(entries) = std::fs::read_dir(user_data) else {
        return dbs;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        // Newer Chromium puts the DB under Network/, older at the profile root.
        for cand in [p.join("Network").join("Cookies"), p.join("Cookies")] {
            if cand.exists() {
                dbs.push(cand);
            }
        }
    }
    dbs
}

/// Strips the 32-byte SHA-256(host) prefix that recent Chromium prepends to the
/// decrypted plaintext, returning a UTF-8 cookie value.
fn chromium_plaintext(bytes: Vec<u8>) -> Option<String> {
    if let Ok(s) = std::str::from_utf8(&bytes) {
        if !s.starts_with(|c: char| c.is_control()) {
            return Some(s.to_string());
        }
    }
    if bytes.len() > 32 {
        if let Ok(s) = std::str::from_utf8(&bytes[32..]) {
            return Some(s.to_string());
        }
    }
    None
}

// ─── Chromium key derivation + decryption (per OS) ────────────────────────────

#[cfg(target_os = "windows")]
fn chromium_key(browser: &ChromiumBrowser) -> Option<Vec<u8>> {
    let local_state = std::fs::read_to_string(browser.user_data.join("Local State")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&local_state).ok()?;
    let b64 = json.get("os_crypt")?.get("encrypted_key")?.as_str()?;
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    // Drop the 5-byte "DPAPI" prefix, then unwrap with the current user's key.
    let wrapped = raw.strip_prefix(b"DPAPI")?;
    dpapi_unprotect(wrapped)
}

#[cfg(target_os = "windows")]
fn dpapi_unprotect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&in_blob, None, None, None, None, 0, &mut out_blob).ok()?;
        let out = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out_blob.pbData as *mut _));
        Some(out)
    }
}

/// AES-256-GCM (`v10`/`v11`) for Windows. `v20` (app-bound) is not decryptable
/// here and returns `None`.
#[cfg(target_os = "windows")]
fn decrypt_chromium(enc: &[u8], key: &[u8]) -> Option<String> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    if enc.len() < 15 || (&enc[..3] != b"v10" && &enc[..3] != b"v11") {
        return None; // v20 app-bound, or unrecognised.
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Nonce::from_slice(&enc[3..15]);
    let plain = cipher.decrypt(nonce, &enc[15..]).ok()?;
    chromium_plaintext(plain)
}

#[cfg(target_os = "macos")]
fn chromium_key(browser: &ChromiumBrowser) -> Option<Vec<u8>> {
    // The AES key is PBKDF2-HMAC-SHA1 of the Keychain "…​Safe Storage" password.
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-w", "-s", browser.safe_storage_service])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let password = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if password.is_empty() {
        return None;
    }
    Some(pbkdf2_key(password.as_bytes(), 1003))
}

#[cfg(target_os = "linux")]
fn chromium_key(_browser: &ChromiumBrowser) -> Option<Vec<u8>> {
    // Best-effort v10 with the well-known password; the libsecret-backed v11
    // key is not read here (Firefox already covers Linux silently).
    Some(pbkdf2_key(b"peanuts", 1))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn pbkdf2_key(password: &[u8], rounds: u32) -> Vec<u8> {
    use sha1::Sha1;
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<Sha1>(password, b"saltysalt", rounds, &mut key);
    key.to_vec()
}

/// AES-128-CBC (`v10`) for macOS/Linux, IV = 16 spaces.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn decrypt_chromium(enc: &[u8], key: &[u8]) -> Option<String> {
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

    if enc.len() < 3 || (&enc[..3] != b"v10" && &enc[..3] != b"v11") {
        return None;
    }
    let iv = [0x20u8; 16];
    let mut buf = enc[3..].to_vec();
    let plain = Aes128CbcDec::new_from_slices(key, &iv)
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .ok()?
        .to_vec();
    chromium_plaintext(plain)
}

// ─── Safari (macOS) ───────────────────────────────────────────────────────────

/// Parses claude.ai cookies out of Safari's `Cookies.binarycookies`.
#[cfg(target_os = "macos")]
fn read_safari() -> Option<Vec<(String, String)>> {
    let home = dirs::home_dir()?;
    let candidates = [
        home.join("Library/Cookies/Cookies.binarycookies"),
        home.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies"),
    ];
    let bytes = candidates
        .iter()
        .find_map(|p| std::fs::read(p).ok())?;
    parse_binarycookies(&bytes)
}

/// Minimal `Cookies.binarycookies` reader: big-endian page table, little-endian
/// cookie records with null-terminated URL/name/value strings at per-record
/// offsets. Returns only claude.ai name/value pairs.
#[cfg(target_os = "macos")]
fn parse_binarycookies(buf: &[u8]) -> Option<Vec<(String, String)>> {
    fn be32(b: &[u8], o: usize) -> Option<usize> {
        b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
    }
    fn le32(b: &[u8], o: usize) -> Option<usize> {
        b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
    }
    fn cstr(b: &[u8], start: usize) -> String {
        let end = b[start..].iter().position(|&c| c == 0).map(|n| start + n).unwrap_or(b.len());
        String::from_utf8_lossy(&b[start..end]).to_string()
    }

    if buf.len() < 8 || &buf[..4] != b"cook" {
        return None;
    }
    let num_pages = be32(buf, 4)?;
    let mut page_sizes = Vec::with_capacity(num_pages);
    let mut off = 8;
    for _ in 0..num_pages {
        page_sizes.push(be32(buf, off)?);
        off += 4;
    }

    let mut cookies = Vec::new();
    let mut page_start = off; // pages begin right after the size table
    for size in page_sizes {
        let page = buf.get(page_start..page_start + size)?;
        // page: [0x00000100 tag][u32 LE num_cookies][num_cookies × u32 LE offset]
        let num_cookies = le32(page, 4)?;
        for i in 0..num_cookies {
            let rec_off = le32(page, 8 + i * 4)?;
            // Cookie record (all offsets relative to the record start):
            //   0:size 4:? 8:flags 12:? 16:url 20:name 24:path 28:value 32:end
            let url_off = le32(page, rec_off + 16)?;
            let name_off = le32(page, rec_off + 20)?;
            let value_off = le32(page, rec_off + 28)?;
            let url = cstr(page, rec_off + url_off);
            if !url.contains("claude.ai") {
                continue;
            }
            let name = cstr(page, rec_off + name_off);
            let value = cstr(page, rec_off + value_off);
            if !name.is_empty() {
                cookies.push((name, value));
            }
        }
        page_start += size;
    }
    Some(cookies)
}
