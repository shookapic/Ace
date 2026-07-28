use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{command, Emitter};

#[derive(Serialize, Clone)]
struct LoginResultEvent {
    provider: String,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

const KEYRING_SERVICE: &str = "ace-app";

struct ProviderConfig {
    id: &'static str,
    client_id: &'static str,
    authorize_url: &'static str,
    token_url: &'static str,
    redirect_uri: &'static str,
    scope: &'static str,
    loopback_port: u16,
}

const OPENAI: ProviderConfig = ProviderConfig {
    id: "openai",
    client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    authorize_url: "https://auth.openai.com/oauth/authorize",
    token_url: "https://auth.openai.com/oauth/token",
    redirect_uri: "http://localhost:1455/auth/callback",
    scope: "openid profile email offline_access",
    loopback_port: 1455,
};

const ANTHROPIC: ProviderConfig = ProviderConfig {
    id: "anthropic",
    client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
    authorize_url: "https://claude.ai/oauth/authorize",
    token_url: "https://platform.claude.com/v1/oauth/token",
    redirect_uri: "http://localhost:53692/callback",
    scope: "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload",
    loopback_port: 53692,
};

fn provider_config(provider: &str) -> Result<&'static ProviderConfig, String> {
    match provider.trim().to_lowercase().as_str() {
        "openai" => Ok(&OPENAI),
        "anthropic" => Ok(&ANTHROPIC),
        _ => Err("unsupported native-OAuth provider".into()),
    }
}

#[derive(Serialize)]
pub struct AuthStatus {
    provider: String,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginLaunchResult {
    provider: String,
    requires_manual_code: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct StoredToken {
    pub(crate) access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
    pub(crate) id_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
}

fn generate_pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build_authorize_url(cfg: &ProviderConfig, challenge: &str, state: &str) -> String {
    if cfg.id == "anthropic" {
        let params = [
            ("code", "true"),
            ("client_id", cfg.client_id),
            ("response_type", "code"),
            ("redirect_uri", cfg.redirect_uri),
            ("scope", cfg.scope),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
        ];
        let mut url = url::Url::parse(cfg.authorize_url).expect("static authorize url is valid");
        url.query_pairs_mut().extend_pairs(params);
        url.to_string()
    } else {
        let mut url = url::Url::parse(cfg.authorize_url).expect("static authorize url is valid");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", cfg.client_id)
            .append_pair("redirect_uri", cfg.redirect_uri)
            .append_pair("scope", cfg.scope)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        url.to_string()
    }
}

const CHUNK_SIZE: usize = 1000;

fn store_token(provider: &str, token: &StoredToken) -> Result<(), String> {
    let payload = serde_json::to_string(token).map_err(|e| e.to_string())?;
    let chunks: Vec<&str> = payload
        .as_bytes()
        .chunks(CHUNK_SIZE)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();

    let count_entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{provider}.chunks"))
        .map_err(|e| format!("failed to open credential store: {e}"))?;
    count_entry
        .set_password(&chunks.len().to_string())
        .map_err(|e| format!("failed to save credential chunk count: {e}"))?;

    for (i, chunk) in chunks.iter().enumerate() {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{provider}.{i}"))
            .map_err(|e| format!("failed to open credential store: {e}"))?;
        entry
            .set_password(chunk)
            .map_err(|e| format!("failed to save credentials (chunk {i}): {e}"))?;
    }

    Ok(())
}

pub(crate) fn load_token(provider: &str) -> Option<StoredToken> {
    let count_entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{provider}.chunks")).ok()?;
    let count: usize = count_entry.get_password().ok()?.parse().ok()?;

    let mut payload = String::new();
    for i in 0..count {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{provider}.{i}")).ok()?;
        payload.push_str(&entry.get_password().ok()?);
    }

    serde_json::from_str(&payload).ok()
}

pub(crate) fn chatgpt_account_id(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(String::from)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn exchange_code(
    cfg: &ProviderConfig,
    code: &str,
    verifier: &str,
    state: &str,
) -> Result<StoredToken, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;

    let response = if cfg.id == "anthropic" {
        let body = serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": cfg.client_id,
            "code": code,
            "state": state,
            "redirect_uri": cfg.redirect_uri,
            "code_verifier": verifier,
        });
        client
            .post(cfg.token_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                format!("token exchange request failed: {e}")
            })?
    } else {
        let fields = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", cfg.redirect_uri),
            ("client_id", cfg.client_id),
            ("code_verifier", verifier),
        ];
        client
            .post(cfg.token_url)
            .form(&fields)
            .send()
            .await
            .map_err(|e| {
                format!("token exchange request failed: {e}")
            })?
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("token exchange failed ({status}): {text}"));
    }

    let parsed: TokenResponse = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse token response: {e}"))?;

    Ok(StoredToken {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at: parsed.expires_in.map(|s| now_secs() + s),
        id_token: parsed.id_token,
    })
}

fn run_loopback_listener(port: u16, expected_state: String) -> Result<(String, String), String> {
    let server = tiny_http::Server::http(("127.0.0.1", port))
        .map_err(|e| {
            format!("failed to bind local callback listener on port {port}: {e}")
        })?;

    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("OAuth callback timed out".into());
        }
        let Some(request) = server
            .recv_timeout(remaining)
            .map_err(|e| format!("callback listener error: {e}"))?
        else {
            continue;
        };
        let raw_url = request.url();

        let url = format!("http://localhost{}", raw_url);
        let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
        
        if parsed.path() != "/callback" && !parsed.path().ends_with("/auth/callback") {
            let response = tiny_http::Response::from_string("Waiting for OAuth redirect...")
                .with_status_code(404);
            let _ = request.respond(response);
            continue;
        }

        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        let code = params.get("code").cloned();
        let state = params.get("state").cloned();

        match (code, state) {
            (Some(code), Some(state)) if state == expected_state => {
                let body = "<!doctype html><html><body style=\"font-family: sans-serif; padding: 40px; text-align: center; background: #111; color: #eee;\"><h2>Ace: sign-in complete</h2><p>You can safely close this window and return to Ace.</p></body></html>";
                let response = tiny_http::Response::from_string(body)
                    .with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap());
                let _ = request.respond(response);
                return Ok((code, state));
            }
            (Some(_), Some(_)) => {
                let body = "<!doctype html><html><body style=\"font-family: sans-serif; padding: 40px; text-align: center; background: #111; color: #eee;\"><h2>Ace: sign-in failed</h2><p>State mismatch. Please try signing in again.</p></body></html>";
                let response = tiny_http::Response::from_string(body)
                    .with_status_code(400)
                    .with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap());
                let _ = request.respond(response);
                return Err("OAuth state mismatch".into());
            }
            _ => {
                let response = tiny_http::Response::from_string("Missing code or state parameters")
                    .with_status_code(400);
                let _ = request.respond(response);
                continue;
            }
        }
    }
}

fn open_in_browser(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    // On Windows, routing through cmd.exe's `start` (which `tauri_plugin_opener`
    // does under the hood) treats unescaped `&` in the URL as a command
    // separator, silently truncating the query string. rundll32's
    // url.dll,FileProtocolHandler opens the URL directly without going
    // through cmd's argument parsing, so query strings survive intact.
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("failed to open browser: {e}"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| format!("failed to open browser: {e}"))
    }
}

#[command]
pub async fn start_oauth_login(
    app: tauri::AppHandle,
    provider: String,
) -> Result<LoginLaunchResult, String> {
    let cfg = provider_config(&provider)?;
    let (verifier, challenge) = generate_pkce();
    let state = generate_state();
    let authorize_url = build_authorize_url(cfg, &challenge, &state);

    open_in_browser(&app, &authorize_url)?;

    let port = cfg.loopback_port;
    let provider_id = cfg.id.to_string();
    let expected_state = state.clone();
    let verifier_for_task = verifier.clone();
    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(move || {
            run_loopback_listener(port, expected_state)
        })
        .await;

        let outcome = match result {
            Ok(Ok((code, state))) => finish_login(&provider_id, &code, &verifier_for_task, &state).await,
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("listener task failed: {e}")),
        };

        let event = match &outcome {
            Ok(()) => {
                LoginResultEvent {
                    provider: provider_id.clone(),
                    success: true,
                    error: None,
                }
            }
            Err(e) => {
                LoginResultEvent {
                    provider: provider_id.clone(),
                    success: false,
                    error: Some(e.clone()),
                }
            }
        };
        let _ = app.emit("auth://login-result", event);
    });

    Ok(LoginLaunchResult {
        provider: cfg.id.to_string(),
        requires_manual_code: false,
    })
}

async fn finish_login(provider: &str, code: &str, verifier: &str, state: &str) -> Result<(), String> {
    let cfg = provider_config(provider)?;
    let token = exchange_code(cfg, code, verifier, state).await?;
    store_token(provider, &token)
}

#[command]
pub fn get_auth_status(provider: String) -> Result<AuthStatus, String> {
    let cfg = provider_config(&provider)?;
    let stored = load_token(cfg.id);

    let available = match &stored {
        Some(token) => match token.expires_at {
            Some(exp) => exp > now_secs() || token.refresh_token.is_some(),
            None => true,
        },
        None => false,
    };

    Ok(AuthStatus {
        provider: cfg.id.to_string(),
        available,
        detail: None,
    })
}

#[command]
pub fn sign_out(provider: String) -> Result<(), String> {
    let cfg = provider_config(&provider)?;

    let count_entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{}.chunks", cfg.id))
        .map_err(|e| format!("failed to open credential store: {e}"))?;
    let count: usize = count_entry
        .get_password()
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    for i in 0..count {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &format!("{}.{i}", cfg.id)) {
            let _ = entry.delete_credential();
        }
    }
    let _ = count_entry.delete_credential();

    Ok(())
}
