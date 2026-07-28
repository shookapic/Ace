use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{command, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio_util::sync::CancellationToken;

use crate::auth::{chatgpt_account_id, load_token};

/// Tracks in-flight chat requests so they can be cancelled by `request_id`.
#[derive(Default)]
pub struct ChatCancels(pub Mutex<HashMap<String, CancellationToken>>);

fn short_timeout_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))
}

fn streaming_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data_base64: String,
}

#[derive(Deserialize, Clone)]
pub struct ChatMessage {
    role: String,
    content: String,
    #[serde(default)]
    attachments: Option<Vec<Attachment>>,
}

fn guess_mime(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "txt" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp" | "h"
        | "html" | "css" | "toml" | "yaml" | "yml" | "sh" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[command]
pub async fn pick_files() -> Result<Vec<Attachment>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let Some(paths) = rfd::FileDialog::new().pick_files() else {
            return Ok(Vec::new());
        };

        paths
            .into_iter()
            .map(|path| {
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment")
                    .to_string();
                Ok(Attachment {
                    name,
                    mime: guess_mime(&path),
                    data_base64: STANDARD.encode(bytes),
                })
            })
            .collect()
    })
    .await
    .map_err(|e| format!("file picker task failed: {e}"))?
}

#[command]
pub async fn transcribe_audio(audio_base64: String, mime: String) -> Result<String, String> {
    let token = load_token("openai")
        .ok_or_else(|| "Connect OpenAI to use voice input.".to_string())?;
    let bytes = STANDARD
        .decode(&audio_base64)
        .map_err(|e| format!("invalid audio data: {e}"))?;

    let ext = if mime.contains("webm") {
        "webm"
    } else if mime.contains("wav") {
        "wav"
    } else if mime.contains("ogg") {
        "ogg"
    } else {
        "m4a"
    };

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(format!("audio.{ext}"))
        .mime_str(&mime)
        .map_err(|e| format!("invalid audio mime type: {e}"))?;
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part("file", part);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;

    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", token.access_token))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("transcription request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        eprintln!("transcribe_audio failed: HTTP {status}");
        return Err(friendly_http_error(status, &text));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse transcription response: {e}"))?;

    parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "no transcription text returned".to_string())
}

fn friendly_http_error(status: reqwest::StatusCode, body: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return format!("Request failed ({status}): {body}");
    };

    let err = parsed.get("error").unwrap_or(&parsed);
    let error_type = err.get("type").and_then(|v| v.as_str());
    let message = err.get("message").and_then(|v| v.as_str());
    let detail = parsed.get("detail").and_then(|v| v.as_str());

    if error_type == Some("usage_limit_reached") {
        let plan = err.get("plan_type").and_then(|v| v.as_str()).unwrap_or("current");
        let resets_in = err.get("resets_in_seconds").and_then(|v| v.as_u64());
        let when = resets_in.map(format_duration).unwrap_or_else(|| "some time".to_string());
        return format!("You've hit your {plan}-plan usage limit. It resets in {when}.");
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return "Too many requests — please wait a bit and try again.".to_string();
    }

    if let Some(detail) = detail {
        return detail.to_string();
    }
    if let Some(message) = message {
        return message.to_string();
    }

    format!("Request failed ({status}): {body}")
}

fn format_duration(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    if days > 0 {
        format!("{days}d {hours}h")
    } else {
        let minutes = (secs % 3600) / 60;
        format!("{hours}h {minutes}m")
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ChunkEvent {
    request_id: String,
    delta: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DoneEvent {
    request_id: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ChatErrorEvent {
    request_id: String,
    error: String,
}

#[derive(Serialize, Clone)]
pub struct ModelInfo {
    id: String,
    label: String,
}

#[command]
pub async fn list_models(provider: String) -> Result<Vec<ModelInfo>, String> {
    let token = load_token(&provider)
        .ok_or_else(|| format!("no stored credentials for {provider} — sign in first"))?;

    match provider.as_str() {
        "openai" => list_openai_models(&token.access_token, &token.id_token).await,
        "anthropic" => list_anthropic_models(&token.access_token).await,
        other => Err(format!("unsupported chat provider: {other}")),
    }
}

async fn list_anthropic_models(access_token: &str) -> Result<Vec<ModelInfo>, String> {
    let client = short_timeout_client()?;
    let response = client
        .get("https://api.anthropic.com/v1/models")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("user-agent", "claude-code/2.1.97")
        .header("x-app", "cli")
        .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await;

    if let Ok(res) = response {
        if res.status().is_success() {
            if let Ok(parsed) = res.json::<serde_json::Value>().await {
                if let Some(models_arr) = parsed.get("data").and_then(|v| v.as_array()) {
                    let mut models = Vec::new();
                    for m in models_arr {
                        if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                            let label = m.get("display_name").and_then(|v| v.as_str()).unwrap_or(id);
                            models.push(ModelInfo {
                                id: id.to_string(),
                                label: label.to_string(),
                            });
                        }
                    }
                    if !models.is_empty() {
                        return Ok(models);
                    }
                }
            }
        }
    }

    Ok(vec![
        ModelInfo { id: "claude-3-7-sonnet-20250219".into(), label: "Claude 3.7 Sonnet (Latest)".into() },
        ModelInfo { id: "claude-3-5-sonnet-20241022".into(), label: "Claude 3.5 Sonnet v2".into() },
        ModelInfo { id: "claude-3-5-haiku-20241022".into(), label: "Claude 3.5 Haiku".into() },
        ModelInfo { id: "claude-3-opus-20240229".into(), label: "Claude 3 Opus".into() },
    ])
}

async fn list_openai_models(access_token: &str, id_token: &Option<String>) -> Result<Vec<ModelInfo>, String> {
    let account_id = id_token
        .as_deref()
        .and_then(chatgpt_account_id)
        .ok_or_else(|| "could not resolve ChatGPT account id from stored credentials".to_string())?;

    let client = short_timeout_client()?;
    let response = client
        .get("https://chatgpt.com/backend-api/codex/models")
        .query(&[("client_version", "0.144.2")])
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("OpenAI-Beta", "responses=v1")
        .header("OpenAI-Originator", "codex")
        .send()
        .await
        .map_err(|e| format!("model list request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(friendly_http_error(status, &text));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse model list: {e}"))?;

    let models = parsed
        .get("models")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(models
        .iter()
        .filter_map(|m| {
            let id = m.get("slug").and_then(|v| v.as_str())?.to_string();
            let label = m
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            Some(ModelInfo { id, label })
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    id: String,
    title: String,
    updated_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMessage {
    role: String,
    content: String,
}

#[command]
pub async fn list_conversations(
    app: tauri::AppHandle,
    provider: String,
) -> Result<Vec<ConversationSummary>, String> {
    // Anthropic history isn't reachable via the inference-scoped OAuth token, and
    // claude.ai's web API is Cloudflare-gated. Instead we reuse the user's own
    // logged-in claude.ai session — captured from Ace's embedded login webview
    // (browser-agnostic) or their Firefox cookie store — including the
    // cf_clearance token, so requests come from the same machine with a
    // pre-cleared session.
    if provider == "anthropic" {
        return list_anthropic_conversations(&app).await;
    }

    let token = load_token(&provider)
        .ok_or_else(|| format!("no stored credentials for {provider} — sign in first"))?;

    match provider.as_str() {
        "openai" => list_openai_conversations(&token.access_token, &token.id_token).await,
        other => Err(format!("unsupported provider: {other}")),
    }
}

#[command]
pub async fn get_conversation(
    app: tauri::AppHandle,
    provider: String,
    conversation_id: String,
) -> Result<Vec<HistoryMessage>, String> {
    if provider == "anthropic" {
        return get_anthropic_conversation(&app, &conversation_id).await;
    }

    let token = load_token(&provider)
        .ok_or_else(|| format!("no stored credentials for {provider} — sign in first"))?;

    match provider.as_str() {
        "openai" => get_openai_conversation(&token.access_token, &token.id_token, &conversation_id).await,
        other => Err(format!("unsupported provider: {other}")),
    }
}

async fn list_openai_conversations(
    access_token: &str,
    id_token: &Option<String>,
) -> Result<Vec<ConversationSummary>, String> {
    let account_id = id_token
        .as_deref()
        .and_then(chatgpt_account_id)
        .ok_or_else(|| "could not resolve ChatGPT account id from stored credentials".to_string())?;

    let client = short_timeout_client()?;
    let response = client
        .get("https://chatgpt.com/backend-api/conversations")
        .query(&[("offset", "0"), ("limit", "25")])
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .send()
        .await
        .map_err(|e| format!("failed to fetch conversations: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(friendly_http_error(status, &text));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse conversations: {e}"))?;

    let items = parsed
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(items
        .iter()
        .filter_map(|c| {
            let id = c.get("id").and_then(|v| v.as_str())?.to_string();
            let title = c
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Untitled")
                .to_string();
            let updated_at = c.get("update_time").and_then(|v| v.as_str()).map(String::from);
            Some(ConversationSummary { id, title, updated_at })
        })
        .collect())
}

async fn get_openai_conversation(
    access_token: &str,
    id_token: &Option<String>,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String> {
    let account_id = id_token
        .as_deref()
        .and_then(chatgpt_account_id)
        .ok_or_else(|| "could not resolve ChatGPT account id from stored credentials".to_string())?;

    let client = short_timeout_client()?;
    let response = client
        .get(format!("https://chatgpt.com/backend-api/conversation/{conversation_id}"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .send()
        .await
        .map_err(|e| format!("failed to fetch conversation: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(friendly_http_error(status, &text));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse conversation: {e}"))?;

    let mapping = parsed
        .get("mapping")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut entries: Vec<(f64, HistoryMessage)> = Vec::new();
    for node in mapping.values() {
        let Some(message) = node.get("message") else { continue };
        let role = message.pointer("/author/role").and_then(|v| v.as_str()).unwrap_or_default();
        if role != "user" && role != "assistant" {
            continue;
        }
        let parts = message
            .pointer("/content/parts")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let text = parts
            .iter()
            .filter_map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        let time = message.get("create_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
        entries.push((time, HistoryMessage { role: role.to_string(), content: text }));
    }
    entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(entries.into_iter().map(|(_, m)| m).collect())
}

const ANTHROPIC_NO_SESSION: &str =
    "Not connected to claude.ai. Click “Connect claude.ai” to sign in, then try again.";

// A fixed browser UA for the embedded login webview. Cloudflare's cf_clearance
// token is keyed to the UA that solved the challenge, so we pin one UA for both
// the webview and the later API replay — guaranteeing they match.
const CLAUDE_WEB_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Clone)]
struct ClaudeSession {
    cookie_header: String,
    org_id: String,
    user_agent: String,
}

/// The claude.ai session captured from Ace's own embedded login webview. This is
/// the browser-agnostic path: the user signs in inside a webview we control, so
/// there's no cookie decryption and no dependency on which browser they use.
#[derive(Default)]
pub struct ClaudeWebSession(std::sync::Mutex<Option<ClaudeSession>>);

fn session_from_cookies(cookies: &[(String, String)]) -> Option<ClaudeSession> {
    if !cookies.iter().any(|(n, _)| n == "sessionKey") {
        return None;
    }
    let org_id = cookies
        .iter()
        .find(|(n, _)| n == "lastActiveOrg")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let cookie_header = cookies
        .iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    Some(ClaudeSession { cookie_header, org_id, user_agent: CLAUDE_WEB_UA.to_string() })
}

/// Reads claude.ai cookies out of the login webview, if it's still around.
/// cookies_for_url deadlocks in sync commands on Windows, so callers must reach
/// this from a spawned thread / blocking task.
fn read_login_webview(app: &tauri::AppHandle) -> Option<ClaudeSession> {
    let window = app.get_webview_window("claude-login")?;
    let url: url::Url = "https://claude.ai".parse().ok()?;
    let cookies = window.cookies_for_url(url).ok()?;
    let pairs: Vec<(String, String)> = cookies
        .iter()
        .map(|c| (c.name().to_string(), c.value().to_string()))
        .collect();
    session_from_cookies(&pairs)
}

/// Opens a real claude.ai login window. Once the user signs in (Cloudflare
/// solved natively, since this is a genuine webview), a background thread grabs
/// the session cookies, caches them, hides the window, and notifies the UI.
#[command]
pub async fn open_claude_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("claude-login") {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::External(
        "https://claude.ai/login".parse().map_err(|_| "invalid claude.ai url".to_string())?,
    );
    WebviewWindowBuilder::new(&app, "claude-login", url)
        .title("Sign in to Claude — Ace")
        .inner_size(480.0, 720.0)
        .user_agent(CLAUDE_WEB_UA)
        .build()
        .map_err(|e| format!("failed to open claude.ai sign-in: {e}"))?;

    let app_bg = app.clone();
    std::thread::spawn(move || {
        for _ in 0..900 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            // Window gone → user closed it; stop polling.
            if app_bg.get_webview_window("claude-login").is_none() {
                return;
            }
            if let Some(session) = read_login_webview(&app_bg) {
                if let Ok(mut guard) = app_bg.state::<ClaudeWebSession>().0.lock() {
                    *guard = Some(session);
                }
                if let Some(w) = app_bg.get_webview_window("claude-login") {
                    let _ = w.hide();
                }
                let _ = app_bg.emit("claude-session://ready", ());
                return;
            }
        }
    });

    Ok(())
}

/// Resolves a usable claude.ai session: prefer a live read from the login
/// webview (freshest cookies), fall back to the cached capture, then to the
/// user's Firefox cookie store.
async fn find_claude_session(app: &tauri::AppHandle) -> Result<ClaudeSession, String> {
    let app_for_read = app.clone();
    if let Ok(Some(session)) =
        tauri::async_runtime::spawn_blocking(move || read_login_webview(&app_for_read)).await
    {
        if let Ok(mut guard) = app.state::<ClaudeWebSession>().0.lock() {
            *guard = Some(session.clone());
        }
        return Ok(session);
    }
    if let Ok(guard) = app.state::<ClaudeWebSession>().0.lock() {
        if let Some(session) = guard.clone() {
            return Ok(session);
        }
    }
    firefox_claude_session()
}

/// Reads the user's own logged-in claude.ai session out of their Firefox cookie
/// store. Firefox keeps cookie values in plaintext (no OS-level encryption), so
/// this needs no decryption — just a read of the profile's cookies.sqlite. The
/// jar already contains a valid `cf_clearance` token (Cloudflare "challenge
/// solved") and the `sessionKey`, so replaying the whole jar from this machine
/// with Firefox's User-Agent reaches claude.ai's web API the same way the
/// browser does.
fn firefox_claude_session() -> Result<ClaudeSession, String> {
    let profiles_dir = dirs::data_dir()
        .map(|d| d.join("Mozilla").join("Firefox").join("Profiles"))
        .filter(|p| p.exists())
        .or_else(|| {
            // On Windows dirs::data_dir() is %APPDATA%; on other platforms the
            // Firefox path differs, but this feature targets the user's Windows box.
            std::env::var_os("APPDATA")
                .map(|a| std::path::Path::new(&a).join("Mozilla").join("Firefox").join("Profiles"))
                .filter(|p| p.exists())
        })
        .ok_or_else(|| ANTHROPIC_NO_SESSION.to_string())?;

    let entries = std::fs::read_dir(&profiles_dir).map_err(|_| ANTHROPIC_NO_SESSION.to_string())?;

    for entry in entries.flatten() {
        let dir = entry.path();
        let cookies_db = dir.join("cookies.sqlite");
        if !cookies_db.exists() {
            continue;
        }

        // Firefox holds a lock and uses WAL; copy the DB to a temp file and open
        // that read-only so we never contend with the running browser.
        let tmp = std::env::temp_dir().join("ace_ff_cookies.sqlite");
        if std::fs::copy(&cookies_db, &tmp).is_err() {
            continue;
        }

        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };

        let Ok(mut stmt) =
            conn.prepare("SELECT name, value FROM moz_cookies WHERE host LIKE '%claude.ai%'")
        else {
            continue;
        };

        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .and_then(|m| m.collect())
            .unwrap_or_default();

        if rows.is_empty() {
            continue;
        }

        let org_id = rows
            .iter()
            .find(|(n, _)| n == "lastActiveOrg")
            .map(|(_, v)| v.clone());
        let has_session = rows.iter().any(|(n, _)| n == "sessionKey");
        if !has_session {
            continue;
        }

        let cookie_header = rows
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; ");

        let user_agent = firefox_user_agent(&dir);

        return Ok(ClaudeSession {
            cookie_header,
            org_id: org_id.unwrap_or_default(),
            user_agent,
        });
    }

    Err(ANTHROPIC_NO_SESSION.to_string())
}

fn firefox_user_agent(profile_dir: &std::path::Path) -> String {
    // cf_clearance is keyed loosely to the UA that earned it; match Firefox's
    // canonical desktop UA using the profile's recorded version.
    let major = std::fs::read_to_string(profile_dir.join("compatibility.ini"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("LastVersion="))
                .and_then(|v| v.split('.').next())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "130".to_string());

    format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:{major}.0) Gecko/20100101 Firefox/{major}.0")
}

async fn claude_web_get(session: &ClaudeSession, url: &str) -> Result<serde_json::Value, String> {
    let client = short_timeout_client()?;
    let response = client
        .get(url)
        .header("User-Agent", &session.user_agent)
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Referer", "https://claude.ai/")
        .header("Cookie", &session.cookie_header)
        .send()
        .await
        .map_err(|e| format!("claude.ai request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        if status.as_u16() == 403 || status.as_u16() == 401 {
            return Err("claude.ai session expired. Click “Connect claude.ai” to sign in again.".to_string());
        }
        return Err(format!("claude.ai returned {status}"));
    }

    response
        .json()
        .await
        .map_err(|e| format!("failed to parse claude.ai response: {e}"))
}

async fn resolve_anthropic_org(session: &ClaudeSession) -> Result<String, String> {
    if !session.org_id.is_empty() {
        return Ok(session.org_id.clone());
    }
    let orgs = claude_web_get(session, "https://claude.ai/api/organizations").await?;
    orgs.as_array()
        .and_then(|a| a.first())
        .and_then(|o| o.get("uuid"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "could not resolve your claude.ai organization".to_string())
}

async fn list_anthropic_conversations(app: &tauri::AppHandle) -> Result<Vec<ConversationSummary>, String> {
    let session = find_claude_session(app).await?;
    let org_id = resolve_anthropic_org(&session).await?;

    let items = claude_web_get(
        &session,
        &format!("https://claude.ai/api/organizations/{org_id}/chat_conversations?limit=40"),
    )
    .await?;

    let arr = items.as_array().cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|c| {
            let id = c.get("uuid").and_then(|v| v.as_str())?.to_string();
            let title = c
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Untitled")
                .to_string();
            let updated_at = c.get("updated_at").and_then(|v| v.as_str()).map(String::from);
            Some(ConversationSummary { id, title, updated_at })
        })
        .collect())
}

async fn get_anthropic_conversation(
    app: &tauri::AppHandle,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String> {
    let session = find_claude_session(app).await?;
    let org_id = resolve_anthropic_org(&session).await?;

    let parsed = claude_web_get(
        &session,
        &format!(
            "https://claude.ai/api/organizations/{org_id}/chat_conversations/{conversation_id}?tree=True&rendering_mode=messages&render_all_tools=true"
        ),
    )
    .await?;

    let messages = parsed
        .get("chat_messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(messages
        .iter()
        .filter_map(|m| {
            let sender = m.get("sender").and_then(|v| v.as_str())?;
            let role = if sender == "human" { "user" } else { "assistant" };

            // Newer messages carry text in content blocks; older ones in `text`.
            let mut text = m.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            if text.trim().is_empty() {
                if let Some(blocks) = m.get("content").and_then(|v| v.as_array()) {
                    text = blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
            if text.trim().is_empty() {
                return None;
            }
            Some(HistoryMessage { role: role.to_string(), content: text })
        })
        .collect())
}

#[command]
pub async fn send_chat_message(
    app: tauri::AppHandle,
    provider: String,
    request_id: String,
    model: Option<String>,
    messages: Vec<ChatMessage>,
) -> Result<(), String> {
    let token = load_token(&provider)
        .ok_or_else(|| format!("no stored credentials for {provider} — sign in first"))?;

    let cancel = CancellationToken::new();
    if let Ok(mut map) = app.state::<ChatCancels>().0.lock() {
        map.insert(request_id.clone(), cancel.clone());
    }

    tauri::async_runtime::spawn(async move {
        let result = match provider.as_str() {
            "openai" => stream_openai(&app, &request_id, &token.access_token, &token.id_token, model.as_deref(), &messages, &cancel).await,
            "anthropic" => stream_anthropic(&app, &request_id, &token.access_token, model.as_deref(), &messages, &cancel).await,
            other => Err(format!("unsupported chat provider: {other}")),
        };

        if let Ok(mut map) = app.state::<ChatCancels>().0.lock() {
            map.remove(&request_id);
        }

        match result {
            Ok(()) => {
                let _ = app.emit("chat://done", DoneEvent { request_id: request_id.clone() });
            }
            Err(e) => {
                eprintln!("chat request failed: {e}");
                let _ = app.emit(
                    "chat://error",
                    ChatErrorEvent { request_id: request_id.clone(), error: e },
                );
            }
        }
    });

    Ok(())
}

/// Cancel an in-flight chat stream; the stream loop breaks and finalizes the
/// partial response via the usual `chat://done` path.
#[command]
pub fn cancel_chat_message(app: tauri::AppHandle, request_id: String) {
    if let Ok(map) = app.state::<ChatCancels>().0.lock() {
        if let Some(token) = map.get(&request_id) {
            token.cancel();
        }
    }
}

fn decode_text_attachment(att: &Attachment) -> Option<String> {
    let bytes = STANDARD.decode(&att.data_base64).ok()?;
    String::from_utf8(bytes).ok()
}

async fn stream_openai(
    app: &tauri::AppHandle,
    request_id: &str,
    access_token: &str,
    id_token: &Option<String>,
    model: Option<&str>,
    messages: &[ChatMessage],
    cancel: &CancellationToken,
) -> Result<(), String> {
    let account_id = id_token
        .as_deref()
        .and_then(chatgpt_account_id)
        .ok_or_else(|| "could not resolve ChatGPT account id from stored credentials".to_string())?;

    let input: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let content_type = if m.role == "user" { "input_text" } else { "output_text" };
            let mut content = vec![serde_json::json!({ "type": content_type, "text": m.content })];

            for att in m.attachments.iter().flatten() {
                if att.mime.starts_with("image/") {
                    content.push(serde_json::json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", att.mime, att.data_base64),
                    }));
                } else if let Some(text) = decode_text_attachment(att) {
                    content.push(serde_json::json!({
                        "type": content_type,
                        "text": format!("[Attached file: {}]\n{}", att.name, text),
                    }));
                }
            }

            serde_json::json!({
                "type": "message",
                "role": m.role,
                "content": content,
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": model.unwrap_or("gpt-5.5"),
        "store": false,
        "stream": true,
        "instructions": "You are a helpful assistant.",
        "input": input,
        "reasoning": { "effort": "medium", "summary": "auto" },
        "text": { "verbosity": "medium" },
    });

    let client = streaming_client()?;
    let response = client
        .post("https://chatgpt.com/backend-api/codex/responses")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("OpenAI-Beta", "responses=v1")
        .header("OpenAI-Originator", "codex")
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("chat request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        eprintln!("chat request failed: HTTP {status}");
        return Err(friendly_http_error(status, &text));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => break,
            next = stream.next() => match next {
                Some(c) => c.map_err(|e| format!("stream read failed: {e}"))?,
                None => break,
            },
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let event = buffer[..pos].to_string();
            buffer.drain(..pos + 2);

            for line in event.lines() {
                let Some(data) = line.strip_prefix("data: ") else { continue };
                if data == "[DONE]" {
                    return Ok(());
                }
                let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else { continue };
                match parsed.get("type").and_then(|v| v.as_str()) {
                    Some("response.output_text.delta") => {
                        if let Some(delta) = parsed.get("delta").and_then(|v| v.as_str()) {
                            let _ = app.emit(
                                "chat://chunk",
                                ChunkEvent { request_id: request_id.to_string(), delta: delta.to_string() },
                            );
                        }
                    }
                    Some("response.completed") | Some("response.done") => return Ok(()),
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

async fn stream_anthropic(
    app: &tauri::AppHandle,
    request_id: &str,
    access_token: &str,
    model: Option<&str>,
    messages: &[ChatMessage],
    cancel: &CancellationToken,
) -> Result<(), String> {
    let system_blocks = vec![
        serde_json::json!({
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude.",
            "cache_control": { "type": "ephemeral" }
        })
    ];

    let formatted_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let attachments: Vec<&Attachment> = m.attachments.iter().flatten().collect();
            if attachments.is_empty() {
                return serde_json::json!({ "role": m.role, "content": m.content });
            }

            let mut blocks: Vec<serde_json::Value> = Vec::new();
            if !m.content.is_empty() {
                blocks.push(serde_json::json!({ "type": "text", "text": m.content }));
            }
            for att in attachments {
                if att.mime.starts_with("image/") {
                    blocks.push(serde_json::json!({
                        "type": "image",
                        "source": { "type": "base64", "media_type": att.mime, "data": att.data_base64 },
                    }));
                } else if att.mime == "application/pdf" {
                    blocks.push(serde_json::json!({
                        "type": "document",
                        "source": { "type": "base64", "media_type": att.mime, "data": att.data_base64 },
                    }));
                } else if let Some(text) = decode_text_attachment(att) {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": format!("[Attached file: {}]\n{}", att.name, text),
                    }));
                }
            }
            serde_json::json!({ "role": m.role, "content": blocks })
        })
        .collect();

    let body = serde_json::json!({
        "model": model.unwrap_or("claude-3-7-sonnet-20250219"),
        "max_tokens": 4096,
        "stream": true,
        "system": system_blocks,
        "messages": formatted_messages,
    });

    let client = streaming_client()?;
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("user-agent", "claude-code/2.1.97")
        .header("x-app", "cli")
        .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14")
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Anthropic chat request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        eprintln!("Anthropic chat request failed: HTTP {status}");
        return Err(friendly_http_error(status, &text));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => break,
            next = stream.next() => match next {
                Some(c) => c.map_err(|e| format!("stream read failed: {e}"))?,
                None => break,
            },
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let event = buffer[..pos].to_string();
            buffer.drain(..pos + 2);

            for line in event.lines() {
                let Some(data) = line.strip_prefix("data: ") else { continue };
                let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else { continue };
                match parsed.get("type").and_then(|v| v.as_str()) {
                    Some("content_block_delta") => {
                        if let Some(delta) = parsed.pointer("/delta/text").and_then(|v| v.as_str()) {
                            let _ = app.emit(
                                "chat://chunk",
                                ChunkEvent { request_id: request_id.to_string(), delta: delta.to_string() },
                            );
                        }
                    }
                    Some("message_stop") => return Ok(()),
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
