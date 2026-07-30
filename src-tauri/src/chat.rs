use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{command, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio_util::sync::CancellationToken;

use crate::auth::{chatgpt_account_id, load_valid_token};

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

/// The file currently shown in the preview window.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPayload {
    name: String,
    language: String,
    content: String,
}

#[derive(Default)]
pub struct PreviewFile(pub Mutex<Option<PreviewPayload>>);

/// Opens (or reuses) a dedicated window showing a file's contents with line
/// numbers and syntax highlighting.
#[command]
pub async fn open_file_preview(
    app: tauri::AppHandle,
    name: String,
    language: String,
    content: String,
) -> Result<(), String> {
    if let Ok(mut guard) = app.state::<PreviewFile>().0.lock() {
        *guard = Some(PreviewPayload { name: name.clone(), language, content });
    }

    if let Some(w) = app.get_webview_window("file-preview") {
        let _ = w.set_title(&format!("{name} — Ace"));
        let _ = w.show();
        let _ = w.set_focus();
        let _ = app.emit_to("file-preview", "preview://update", ());
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, "file-preview", WebviewUrl::App("index.html".into()))
        .title(format!("{name} — Ace"))
        .inner_size(820.0, 620.0)
        .min_inner_size(420.0, 320.0)
        .resizable(true)
        .decorations(false)
        .build()
        .map_err(|e| format!("failed to open preview window: {e}"))?;
    Ok(())
}

/// Returns the file the preview window should display.
#[command]
pub fn get_preview_file(app: tauri::AppHandle) -> Option<PreviewPayload> {
    app.state::<PreviewFile>().0.lock().ok().and_then(|g| g.clone())
}

/// Writes HTML to a temp file and opens it in the user's default browser so they
/// can run/preview a generated page (e.g. a game).
#[command]
pub async fn open_html_in_browser(
    app: tauri::AppHandle,
    name: String,
    content: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = std::env::temp_dir().join("ace_previews");
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to prepare preview dir: {e}"))?;
    // Sanitise the filename to a safe basename.
    let safe: String = basename(&name)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let safe = if safe.is_empty() { "preview.html".to_string() } else { safe };
    let path = dir.join(&safe);
    std::fs::write(&path, content).map_err(|e| format!("failed to write preview: {e}"))?;
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("failed to open preview: {e}"))?;
    Ok(())
}

/// Saves generated text content to a user-chosen path (for file-artifact downloads).
#[command]
pub async fn save_file(name: String, content: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new().set_file_name(&name).save_file() else {
            return Ok(false);
        };
        std::fs::write(&path, content).map_err(|e| format!("failed to save {name}: {e}"))?;
        Ok(true)
    })
    .await
    .map_err(|e| format!("save task failed: {e}"))?
}

#[command]
pub async fn transcribe_audio(audio_base64: String, mime: String) -> Result<String, String> {
    let token = load_valid_token("openai")
        .await
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

/// Emitted for the experimental claude.ai write-back path so the frontend knows
/// which server-side conversation the turn was saved into (matters when a brand
/// new conversation was just created for it).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WebSavedEvent {
    request_id: String,
    conversation_id: String,
}

/// Token usage reported by the inference APIs, surfaced per assistant message.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UsageEvent {
    request_id: String,
    input_tokens: u64,
    output_tokens: u64,
    model: String,
}

/// A tool step claude.ai ran server-side (e.g. "Creating file", "Running
/// command"), surfaced so the UI isn't silent during tool-call gaps.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ToolEvent {
    request_id: String,
    label: String,
}

/// A file claude.ai created/edited server-side, reconstructed from the stream.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FileArtifact {
    name: String,
    language: String,
    content: String,
}

/// The final set of files claude.ai presented at the end of a response.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FilesEvent {
    request_id: String,
    files: Vec<FileArtifact>,
}

/// Info claude.ai's web stream exposes instead of token usage: the model, and
/// the account's plan-usage windows (fraction 0..1 of the 5h / 7d limits).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WebInfoEvent {
    request_id: String,
    model: String,
    usage5h: f64,
    usage7d: f64,
}

/// Best-effort syntax-highlight language from a filename.
fn language_for(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower == "makefile" || lower.ends_with(".mk") {
        return "makefile".into();
    }
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "c" | "h" => "c",
        "hpp" | "hh" | "hxx" | "cpp" | "cc" | "cxx" | "c++" => "cpp",
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" => "bash",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        _ => "",
    }
    .to_string()
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Extracts triple-quoted (`'''` / `"""`) string literals from python source.
fn triple_quoted_strings(code: &str) -> Vec<String> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 3 <= b.len() {
        let is_sq = &b[i..i + 3] == b"'''";
        let is_dq = &b[i..i + 3] == b"\"\"\"";
        if is_sq || is_dq {
            let delim: &[u8] = if is_sq { b"'''" } else { b"\"\"\"" };
            let start = i + 3;
            let mut j = start;
            let mut found = None;
            while j + 3 <= b.len() {
                if &b[j..j + 3] == delim {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            match found {
                Some(end) => {
                    if let Ok(s) = std::str::from_utf8(&b[start..end]) {
                        out.push(s.to_string());
                    }
                    i = end + 3;
                }
                None => break,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Removes common leading whitespace (mimics python's textwrap.dedent) and a
/// single leading newline, so `dedent('''\n<html>...''')` comes out clean.
fn dedent_str(s: &str) -> String {
    let s = s.strip_prefix('\n').unwrap_or(s);
    let indent = s
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    s.split('\n')
        .map(|l| if l.len() >= indent { &l[indent..] } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Best-effort reconstruction of the files ChatGPT's code-interpreter wrote to
/// the sandbox: the file contents live as triple-quoted string literals in the
/// python code. Pairs them with the presented filenames.
fn reconstruct_python_files(code: &str, names: &[String]) -> Vec<FileArtifact> {
    let strings = triple_quoted_strings(code);
    if strings.is_empty() || names.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    if names.len() == 1 {
        // Single file → the largest literal is its content.
        if let Some(content) = strings.iter().max_by_key(|s| s.len()) {
            let name = basename(&names[0]);
            out.push(FileArtifact { language: language_for(&name), name, content: dedent_str(content) });
        }
    } else {
        // Multiple files → pair with literals in appearance order (best effort).
        for (name, content) in names.iter().zip(strings.iter()) {
            let name = basename(name);
            out.push(FileArtifact { language: language_for(&name), name, content: dedent_str(content) });
        }
    }
    out.into_iter().filter(|f| !f.content.trim().is_empty()).collect()
}

#[derive(Serialize, Clone)]
pub struct ModelInfo {
    id: String,
    label: String,
}

#[command]
pub async fn list_models(provider: String) -> Result<Vec<ModelInfo>, String> {
    let token = load_valid_token(&provider)
        .await
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

    // Fallback only when the live /v1/models call can't be reached. Uses undated
    // aliases so retired dated snapshots (which 404) don't get pinned here.
    Ok(vec![
        ModelInfo { id: "claude-sonnet-4-5".into(), label: "Claude Sonnet 4.5".into() },
        ModelInfo { id: "claude-opus-4-1".into(), label: "Claude Opus 4.1".into() },
        ModelInfo { id: "claude-3-5-haiku-latest".into(), label: "Claude Haiku 3.5".into() },
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

    let token = load_valid_token(&provider)
        .await
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

    let token = load_valid_token(&provider)
        .await
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
    // Silent fast-path: read the user's own logged-in session out of whichever
    // browser has one. Firefox is cheapest (plaintext); Chrome/Edge/Brave and
    // Safari need decryption, so try them only if Firefox has nothing.
    firefox_claude_session().or_else(|_| other_browser_claude_session())
}

/// Silent claude.ai session from a non-Firefox browser (Chromium family or
/// Safari). See [`crate::browser_cookies`] for the per-browser decryption.
fn other_browser_claude_session() -> Result<ClaudeSession, String> {
    let jar = crate::browser_cookies::find_claude_jar()
        .ok_or_else(|| ANTHROPIC_NO_SESSION.to_string())?;
    let mut session =
        session_from_cookies(&jar.cookies).ok_or_else(|| ANTHROPIC_NO_SESSION.to_string())?;
    session.user_agent = jar.user_agent;
    Ok(session)
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

// ---------------------------------------------------------------------------
// EXPERIMENTAL: claude.ai write-back.
//
// The normal chat path talks to the stateless inference API (api.anthropic.com)
// — nothing lands in the user's claude.ai account. This path instead POSTs to
// claude.ai's own undocumented `/completion` endpoint using the same session
// cookies we already read history with, so claude.ai's backend runs the model
// AND persists both turns into the real account conversation.
//
// It is fragile by nature: the endpoint/SSE shape is undocumented and can change
// without notice, and it forfeits Ace's model/provider control (claude.ai's
// server picks the model). Gated behind an opt-in toggle in the UI.
// ---------------------------------------------------------------------------

/// Sentinel parent used by claude.ai for the first message in a conversation.
const CLAUDE_ROOT_PARENT: &str = "00000000-0000-4000-8000-000000000000";

/// Appends a raw SSE frame to a debug log (temp dir) when ACE_DEBUG_STREAM is set.
/// Used to capture claude.ai's undocumented tool/artifact event format.
fn debug_stream_append(line: &str) {
    let path = std::env::temp_dir().join("ace_claude_stream.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
}

fn new_uuid_v4() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// Applies the shared claude.ai browser-session headers to a request.
fn claude_web_headers(rb: reqwest::RequestBuilder, session: &ClaudeSession) -> reqwest::RequestBuilder {
    rb.header("User-Agent", &session.user_agent)
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Referer", "https://claude.ai/")
        .header("Origin", "https://claude.ai")
        .header("Cookie", &session.cookie_header)
}

async fn claude_web_post_json(
    session: &ClaudeSession,
    url: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = short_timeout_client()?;
    let response = claude_web_headers(client.post(url), session)
        .header("Content-Type", "application/json")
        .json(&body)
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

/// Creates a fresh, empty claude.ai conversation and returns its uuid.
async fn create_anthropic_conversation(session: &ClaudeSession, org_id: &str) -> Result<String, String> {
    let uuid = new_uuid_v4();
    let body = serde_json::json!({ "uuid": uuid, "name": "" });
    let created = claude_web_post_json(
        session,
        &format!("https://claude.ai/api/organizations/{org_id}/chat_conversations"),
        body,
    )
    .await?;
    Ok(created
        .get("uuid")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or(uuid))
}

/// Resolves the parent uuid to hang the new turn off of — the conversation's
/// current leaf message. Getting this wrong (e.g. defaulting to the root) makes
/// claude.ai branch the turn as an alternate of the first message instead of
/// continuing the thread, so it never shows up in the linear conversation.
///
/// Prefers the server's own `current_leaf_message_uuid`; when that's absent
/// (claude.ai often omits it) falls back to the newest message in the tree.
/// Only an empty/unreadable conversation resolves to the root sentinel.
async fn anthropic_leaf_uuid(session: &ClaudeSession, org_id: &str, conv_id: &str) -> String {
    let root = CLAUDE_ROOT_PARENT.to_string();
    let Ok(v) = claude_web_get(
        session,
        &format!(
            "https://claude.ai/api/organizations/{org_id}/chat_conversations/{conv_id}?tree=True&rendering_mode=messages"
        ),
    )
    .await
    else {
        return root;
    };

    if let Some(leaf) = v
        .get("current_leaf_message_uuid")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        return leaf.to_string();
    }

    // Newest message by created_at (ISO-8601 sorts lexicographically). In a
    // linear chat that's the leaf; parent the new turn onto it.
    v.get("chat_messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|m| m.get("created_at").and_then(|c| c.as_str()).unwrap_or("").to_string())
        })
        .and_then(|m| m.get("uuid"))
        .and_then(|x| x.as_str())
        .map(String::from)
        .unwrap_or(root)
}

/// Streams claude.ai's `/completion` SSE, emitting `chat://chunk` deltas exactly
/// like the inference path so the existing frontend listeners handle it unchanged.
async fn stream_claude_web(
    app: &tauri::AppHandle,
    request_id: &str,
    session: &ClaudeSession,
    org_id: &str,
    conv_id: &str,
    parent: &str,
    prompt: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let body = serde_json::json!({
        "prompt": prompt,
        "parent_message_uuid": parent,
        "timezone": "UTC",
        "attachments": [],
        "files": [],
        "sync_sources": [],
        "rendering_mode": "messages",
    });

    let client = streaming_client()?;
    let response = claude_web_headers(
        client.post(format!(
            "https://claude.ai/api/organizations/{org_id}/chat_conversations/{conv_id}/completion"
        )),
        session,
    )
    .header("Content-Type", "application/json")
    .header("Accept", "text/event-stream")
    .json(&body)
    .send()
    .await
    .map_err(|e| format!("claude.ai completion request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        eprintln!("claude.ai completion failed: HTTP {status}");
        if status.as_u16() == 403 || status.as_u16() == 401 {
            return Err("claude.ai session expired. Click “Connect claude.ai” to sign in again.".to_string());
        }
        return Err(format!("claude.ai returned {status}"));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    // Opt-in raw-stream capture (set ACE_DEBUG_STREAM=1) so we can inspect
    // claude.ai's tool_use / artifact event shapes.
    let debug_log = std::env::var("ACE_DEBUG_STREAM").is_ok();
    if debug_log {
        debug_stream_append("===== new claude.ai response =====");
    }

    // claude.ai runs server-side tools (create_file / str_replace / bash_tool /
    // present_files). We reconstruct the files it builds from the tool inputs so
    // Ace can render them, and surface each tool step so gaps aren't silent.
    // index -> (tool name, accumulated input JSON)
    let mut tool_blocks: HashMap<u64, (String, String)> = HashMap::new();
    // basename -> file content (the reconstructed virtual filesystem)
    let mut files: HashMap<String, String> = HashMap::new();
    // claude.ai reports model + plan-usage windows instead of token counts.
    let mut web_model = String::new();
    let mut usage_5h = 0f64;
    let mut usage_7d = 0f64;

    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => break,
            next = stream.next() => match next {
                Some(c) => c.map_err(|e| format!("stream read failed: {e}"))?,
                None => break,
            },
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // claude.ai emits one `data:` line per SSE frame.
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].to_string();
            buffer.drain(..pos + 1);

            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if debug_log {
                debug_stream_append(data);
            }
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else { continue };

            match parsed.get("type").and_then(|v| v.as_str()) {
                Some("content_block_start") => {
                    let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    let block = parsed.get("content_block");
                    match block.and_then(|b| b.get("type")).and_then(|v| v.as_str()) {
                        Some("tool_use") => {
                            let name = block
                                .and_then(|b| b.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            tool_blocks.insert(index, (name.clone(), String::new()));
                            // Surface the step label ("Creating file", "Running command"…).
                            if let Some(label) =
                                block.and_then(|b| b.get("message")).and_then(|v| v.as_str())
                            {
                                if !label.is_empty() {
                                    let _ = app.emit(
                                        "chat://tool",
                                        ToolEvent {
                                            request_id: request_id.to_string(),
                                            label: label.to_string(),
                                        },
                                    );
                                }
                            }
                        }
                        Some("tool_result") => {
                            let name =
                                block.and_then(|b| b.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                            if name == "present_files" {
                                emit_presented_files(app, request_id, block, &files);
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_delta") => {
                    let delta = parsed.get("delta");
                    match delta.and_then(|d| d.get("type")).and_then(|v| v.as_str()) {
                        Some("text_delta") => {
                            if let Some(d) = delta.and_then(|d| d.get("text")).and_then(|v| v.as_str()) {
                                if !d.is_empty() {
                                    let _ = app.emit(
                                        "chat://chunk",
                                        ChunkEvent { request_id: request_id.to_string(), delta: d.to_string() },
                                    );
                                }
                            }
                        }
                        Some("input_json_delta") => {
                            let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                            if let Some((_, buf)) = tool_blocks.get_mut(&index) {
                                if let Some(frag) =
                                    delta.and_then(|d| d.get("partial_json")).and_then(|v| v.as_str())
                                {
                                    buf.push_str(frag);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_stop") => {
                    let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some((name, input)) = tool_blocks.remove(&index) {
                        apply_file_tool(&name, &input, &mut files);
                    }
                }
                Some("message_start") => {
                    if let Some(m) = parsed.pointer("/message/model").and_then(|v| v.as_str()) {
                        web_model = m.to_string();
                    }
                }
                Some("message_limit") => {
                    if let Some(u) =
                        parsed.pointer("/message_limit/windows/5h/utilization").and_then(|v| v.as_f64())
                    {
                        usage_5h = u;
                    }
                    if let Some(u) =
                        parsed.pointer("/message_limit/windows/7d/utilization").and_then(|v| v.as_f64())
                    {
                        usage_7d = u;
                    }
                }
                // Classic claude.ai SSE also delivers plain text under `completion`.
                _ => {
                    if let Some(d) = parsed.get("completion").and_then(|v| v.as_str()) {
                        if !d.is_empty() {
                            let _ = app.emit(
                                "chat://chunk",
                                ChunkEvent { request_id: request_id.to_string(), delta: d.to_string() },
                            );
                        }
                    }
                }
            }
        }
    }

    if !web_model.is_empty() {
        let _ = app.emit(
            "chat://webinfo",
            WebInfoEvent {
                request_id: request_id.to_string(),
                model: web_model,
                usage5h: usage_5h,
                usage7d: usage_7d,
            },
        );
    }

    Ok(())
}

/// Applies a completed file tool's input to the reconstructed filesystem.
fn apply_file_tool(name: &str, input: &str, files: &mut HashMap<String, String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(input) else { return };
    match name {
        "create_file" => {
            if let (Some(path), Some(text)) = (
                v.get("path").and_then(|p| p.as_str()),
                v.get("file_text").and_then(|t| t.as_str()),
            ) {
                files.insert(basename(path), text.to_string());
            }
        }
        "str_replace" => {
            if let (Some(path), Some(old), Some(new)) = (
                v.get("path").and_then(|p| p.as_str()),
                v.get("old_str").and_then(|t| t.as_str()),
                v.get("new_str").and_then(|t| t.as_str()),
            ) {
                let key = basename(path);
                if let Some(content) = files.get_mut(&key) {
                    if let Some(pos) = content.find(old) {
                        content.replace_range(pos..pos + old.len(), new);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Emits the final presented file set, pulling reconstructed content by basename.
fn emit_presented_files(
    app: &tauri::AppHandle,
    request_id: &str,
    block: Option<&serde_json::Value>,
    files: &HashMap<String, String>,
) {
    let Some(content) = block.and_then(|b| b.get("content")).and_then(|v| v.as_array()) else {
        return;
    };
    let mut out = Vec::new();
    for item in content {
        let Some(path) = item.get("file_path").and_then(|v| v.as_str()) else { continue };
        let name = basename(path);
        let body = files.get(&name).cloned().unwrap_or_default();
        if body.is_empty() {
            continue; // no reconstructed content (e.g. a binary) — skip
        }
        out.push(FileArtifact { language: language_for(&name), name, content: body });
    }
    if !out.is_empty() {
        let _ = app.emit(
            "chat://files",
            FilesEvent { request_id: request_id.to_string(), files: out },
        );
    }
}

/// EXPERIMENTAL: send a turn into the user's real claude.ai account conversation.
/// If `conversation_id` is None/empty a new claude.ai conversation is created.
/// Emits the same `chat://chunk|done|error` events as `send_chat_message`, plus a
/// one-shot `chat://web-saved` carrying the resolved conversation id.
#[command]
pub async fn send_claude_web_message(
    app: tauri::AppHandle,
    request_id: String,
    conversation_id: Option<String>,
    prompt: String,
) -> Result<(), String> {
    let session = find_claude_session(&app).await?;
    let org_id = resolve_anthropic_org(&session).await?;

    let conv_id = match conversation_id {
        Some(id) if !id.is_empty() => id,
        _ => create_anthropic_conversation(&session, &org_id).await?,
    };
    let parent = anthropic_leaf_uuid(&session, &org_id, &conv_id).await;

    let cancel = CancellationToken::new();
    if let Ok(mut map) = app.state::<ChatCancels>().0.lock() {
        map.insert(request_id.clone(), cancel.clone());
    }

    let _ = app.emit(
        "chat://web-saved",
        WebSavedEvent { request_id: request_id.clone(), conversation_id: conv_id.clone() },
    );

    tauri::async_runtime::spawn(async move {
        let result =
            stream_claude_web(&app, &request_id, &session, &org_id, &conv_id, &parent, &prompt, &cancel)
                .await;

        if let Ok(mut map) = app.state::<ChatCancels>().0.lock() {
            map.remove(&request_id);
        }

        match result {
            Ok(()) => {
                let _ = app.emit("chat://done", DoneEvent { request_id: request_id.clone() });
            }
            Err(e) => {
                eprintln!("claude.ai write-back failed: {e}");
                let _ = app.emit(
                    "chat://error",
                    ChatErrorEvent { request_id: request_id.clone(), error: e },
                );
            }
        }
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// EXPERIMENTAL: ChatGPT (chatgpt.com) write-back.
//
// Unlike claude.ai's open `/completion`, ChatGPT's `/backend-api/conversation`
// endpoint is gated by OpenAI's "sentinel" anti-automation system: every send
// needs a chat-requirements token and, usually, a proof-of-work answer. We mint
// both, then POST the turn so ChatGPT's backend runs the model and persists it
// into the real account conversation.
//
// This is the fragile part of the app: the sentinel scheme, PoW config, and the
// streaming delta protocol are all undocumented and change without notice.
// ---------------------------------------------------------------------------

const OPENAI_WEB_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// The browser-fingerprint config array hashed for the proof-of-work. OpenAI's
/// server only checks the resulting hash difficulty, not the field values, so
/// plausible constants work — only slot 3 (the iteration counter) varies.
fn openai_pow_config(index: u64) -> String {
    let config = serde_json::json!([
        8000,
        "Tue Jan 01 2025 00:00:00 GMT+0000 (Coordinated Universal Time)",
        4294705152u64,
        index,
        OPENAI_WEB_UA,
        "https://cdn.oaistatic.com/assets/chat.js",
        "prod",
        "en-US",
        "en-US,en",
        0,
        "plugins-[object PluginArray]",
        "_reactListeningx",
        "mousemove"
    ]);
    STANDARD.encode(serde_json::to_string(&config).unwrap_or_default())
}

/// Hashcash-style PoW: find an iteration whose SHA3-512(seed + base64(config))
/// hex digest sorts at or below `difficulty`. Returns the `gAAAAAB…` proof token.
fn openai_solve_pow(seed: &str, difficulty: &str) -> String {
    use sha3::{Digest, Sha3_512};
    for i in 0..500_000u64 {
        let encoded = openai_pow_config(i);
        let mut hasher = Sha3_512::new();
        hasher.update(seed.as_bytes());
        hasher.update(encoded.as_bytes());
        let hex = hex_encode(&hasher.finalize());
        if hex.len() >= difficulty.len() && hex[..difficulty.len()] <= *difficulty {
            return format!("gAAAAAB{encoded}");
        }
    }
    // Couldn't solve in budget — send an unsolved token; low-difficulty servers
    // sometimes still accept it, otherwise the send errors clearly.
    format!("gAAAAAB{}", openai_pow_config(0))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn openai_common_headers(
    rb: reqwest::RequestBuilder,
    access_token: &str,
    account_id: &str,
    cookie: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut rb = rb
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("User-Agent", OPENAI_WEB_UA)
        .header("oai-language", "en-US")
        .header("Origin", "https://chatgpt.com")
        .header("Referer", "https://chatgpt.com/");
    // A cf_clearance-bearing browser session (captured from the verification
    // webview) is what lets the sentinel skip the Turnstile CAPTCHA.
    if let Some(c) = cookie {
        rb = rb.header("Cookie", c);
    }
    rb
}

// ---- ChatGPT browser session (for Cloudflare/Turnstile clearance) ----

#[derive(Clone)]
struct ChatGptSession {
    cookie_header: String,
}

/// A chatgpt.com session captured from Ace's own verification webview — carries
/// the `cf_clearance` cookie so API calls clear Cloudflare/Turnstile.
#[derive(Default)]
pub struct ChatGptWebSession(std::sync::Mutex<Option<ChatGptSession>>);

/// Reads chatgpt.com cookies out of the verification webview once Cloudflare has
/// been cleared (i.e. a `cf_clearance` cookie exists).
fn read_chatgpt_login_webview(app: &tauri::AppHandle) -> Option<ChatGptSession> {
    let window = app.get_webview_window("chatgpt-login")?;
    let url: url::Url = "https://chatgpt.com".parse().ok()?;
    let cookies = window.cookies_for_url(url).ok()?;
    if !cookies.iter().any(|c| c.name() == "cf_clearance") {
        return None;
    }
    let cookie_header = cookies
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");
    Some(ChatGptSession { cookie_header })
}

/// Opens a real chatgpt.com window so the user can clear the Cloudflare/Turnstile
/// challenge natively; a background thread then captures the session cookies.
#[command]
pub async fn open_chatgpt_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("chatgpt-login") {
        // Bring it back on-screen in case it was parked off-screen after signing in.
        let _ = existing.set_skip_taskbar(false);
        let _ = existing.center();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::External(
        "https://chatgpt.com/".parse().map_err(|_| "invalid chatgpt.com url".to_string())?,
    );
    // Opt-in raw-stream capture for studying ChatGPT's canvas/file events.
    let script = if std::env::var("ACE_DEBUG_STREAM").is_ok() {
        format!("{ACE_CHATGPT_SCRIPT}\nwindow.__aceDebug=true;")
    } else {
        ACE_CHATGPT_SCRIPT.to_string()
    };

    let app_nav = app.clone();
    WebviewWindowBuilder::new(&app, "chatgpt-login", url)
        .title("Verify ChatGPT — Ace")
        .inner_size(480.0, 720.0)
        .user_agent(OPENAI_WEB_UA)
        .initialization_script(&script)
        // The injected script signals Ace by navigating to https://ace.relay/… —
        // intercept those, act on them, and cancel (false) so the page stays put.
        // Everything else navigates normally (true).
        .on_navigation(move |u| match u.as_str().strip_prefix("https://ace.relay/") {
            Some(rest) => {
                handle_ace_relay(&app_nav, rest);
                false
            }
            None => true,
        })
        .build()
        .map_err(|e| format!("failed to open ChatGPT verification: {e}"))?;

    Ok(())
}


// ---- Webview-driven ChatGPT send (uses OpenAI's own token machinery) ----
//
// OpenAI's sentinel Turnstile can't be satisfied by replaying the API from Rust:
// every message needs a fresh token minted by OpenAI's own obfuscated in-page
// code. So for ChatGPT we drive the real chatgpt.com page: an injected script
// types our prompt into the composer and sends it (OpenAI's code mints all the
// tokens), then tees the `/conversation` SSE response back to Ace via IPC.

/// Injected at document-start into the chatgpt.com webview. Signals Ace by
/// navigating to `https://ace.relay/…` (which Rust intercepts and cancels) —
/// this avoids remote-origin IPC, which the page's CSP/allowlist blocks. Parses
/// the conversation SSE in-page and streams the text back over that channel.
const ACE_CHATGPT_SCRIPT: &str = r#"
(function () {
  if (window.__aceInstalled) return;
  window.__aceInstalled = true;
  function enc(s) {
    try { return btoa(unescape(encodeURIComponent(s))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, ''); }
    catch (e) { return ''; }
  }
  // Signal Rust via an intercepted (and cancelled) navigation.
  function sig(kind, id, obj) {
    try { window.location.href = 'https://ace.relay/' + kind + '/' + (id || 'x') + '/' + enc(JSON.stringify(obj || {})); }
    catch (e) {}
  }

  var cur = null; // { id, text, conv, sent }
  var flushTimer = null;
  function startFlush() {
    if (flushTimer) return;
    // Push cumulative text on a timer so rapid deltas collapse into paced,
    // reliably-delivered navigations (successive sync location sets would drop).
    flushTimer = setInterval(function () {
      if (cur && cur.text !== cur.sent) { cur.sent = cur.text; sig('chunk', cur.id, { text: cur.text, conv: cur.conv }); }
    }, 150);
  }
  function stopFlush() { if (flushTimer) { clearInterval(flushTimer); flushTimer = null; } }

  // ChatGPT's v1 delta protocol streams several message nodes (system, the
  // python code-interpreter tool, execution output, then the visible answer).
  // Only the assistant message with recipient "all" + content_type "text" is the
  // user-facing reply — everything else (code etc.) must NOT be shown as text.
  function isVisibleMsg(m) {
    return !!(m && m.author && m.author.role === 'assistant'
      && m.recipient === 'all'
      && m.content && m.content.content_type === 'text');
  }
  function isCodeMsg(m) {
    return !!(m && m.author && m.author.role === 'assistant'
      && m.content && m.content.content_type === 'code');
  }
  function applyNode(m, convId) {
    if (convId && !cur.conv) cur.conv = convId;
    // Track which stream the current message writes into.
    cur.mode = isVisibleMsg(m) ? 'text' : (isCodeMsg(m) ? 'code' : 'other');
    if (cur.mode === 'text') {
      var parts = (m.content && m.content.parts) || [];
      var t = parts.join('');
      if (t) cur.text += t; // initial content (usually empty)
    }
  }
  function feedStr(s, path) {
    if (cur.mode === 'text' && (!path || path.indexOf('/parts/') !== -1)) cur.text += s;
    // Code-interpreter code streams into /content/text — capture it so we can
    // reconstruct any file it writes to the sandbox.
    else if (cur.mode === 'code' && (!path || path.indexOf('/text') !== -1)) cur.code += s;
  }
  function feed(d) {
    if (!cur) return;
    var o; try { o = JSON.parse(d); } catch (e) { return; }
    var val = o.v, path = o.p;
    if (val && typeof val === 'object' && val.message) { applyNode(val.message, val.conversation_id); return; }
    if (typeof val === 'string') { feedStr(val, path); return; }
    if (Array.isArray(val)) {
      for (var i = 0; i < val.length; i++) {
        var sub = val[i];
        if (sub && sub.v && typeof sub.v === 'object' && sub.v.message) { applyNode(sub.v.message, sub.v.conversation_id); }
        else if (sub && typeof sub.v === 'string') { feedStr(sub.v, sub.p); }
      }
      return;
    }
    if (o.conversation_id && !cur.conv) cur.conv = o.conversation_id;
  }

  var origFetch = window.fetch;
  window.fetch = function () {
    var p = origFetch.apply(this, arguments);
    // Tee any event-stream response while a send is in flight — matched by
    // content-type, so the conversation endpoint's path doesn't matter.
    if (window.__aceReq) {
      p.then(function (res) {
        try {
          var ct = (res.headers.get('content-type') || '');
          if (ct.indexOf('event-stream') === -1 || !window.__aceReq) return;
          cur = { id: window.__aceReq, text: '', code: '', conv: null, sent: '', raw: '', mode: 'other' };
          window.__aceReq = null;
          startFlush();
          var reader = res.clone().body.getReader();
          var dec = new TextDecoder();
          var buf = '';
          (function pump() {
            reader.read().then(function (r) {
              if (r.done) {
                stopFlush();
                // The last SSE line can arrive without a trailing newline before
                // the stream closes — feed whatever is still buffered so the final
                // sentence isn't dropped.
                buf.split('\n').forEach(function (line) {
                  if (line.indexOf('data:') === 0) { var d = line.slice(5).trim(); if (d && d !== '[DONE]') feed(d); }
                });
                // Opt-in: ship the whole raw SSE so we can study canvas/file events.
                if (window.__aceDebug && cur.raw) { sig('debug', cur.id, { raw: cur.raw }); }
                // Reconstruct sandbox files ChatGPT wrote from the code, if any.
                var names = [];
                var re = /sandbox:\/mnt\/data\/([^)\s"'<>]+)/g, mt;
                while ((mt = re.exec(cur.text)) !== null) { if (names.indexOf(mt[1]) === -1) names.push(mt[1]); }
                if (names.length && cur.code) { sig('files', cur.id, { code: cur.code, names: names }); }
                var payload = { text: cur.text, conv: cur.conv };
                sig('done', cur.id, payload);
                // Re-assert once in case the done navigation raced the last chunk.
                setTimeout(function () { sig('done', cur.id, payload); }, 200);
                return;
              }
              var textChunk = dec.decode(r.value, { stream: true });
              if (window.__aceDebug) { cur.raw += textChunk; }
              buf += textChunk;
              var i;
              while ((i = buf.indexOf('\n')) !== -1) {
                var line = buf.slice(0, i); buf = buf.slice(i + 1);
                if (line.indexOf('data:') === 0) { var d = line.slice(5).trim(); if (d && d !== '[DONE]') feed(d); }
              }
              pump();
            }).catch(function (e) { stopFlush(); sig('error', cur.id, { text: String(e) }); });
          })();
        } catch (e) {}
      });
    }
    return p;
  };

  window.__aceSend = function (requestId, text) {
    window.__aceReq = requestId;
    var attempts = 0;
    (function trySend() {
      var ta = document.getElementById('prompt-textarea')
        || document.querySelector('div[contenteditable="true"]')
        || document.querySelector('textarea');
      if (!ta) {
        if (attempts++ < 40) { setTimeout(trySend, 200); return; }
        sig('error', requestId, { text: 'ChatGPT composer not found — is the page signed in?' });
        return;
      }
      ta.focus();
      if (ta.getAttribute('contenteditable') === 'true') {
        ta.innerHTML = '';
        document.execCommand('insertText', false, text);
        ta.dispatchEvent(new InputEvent('input', { bubbles: true }));
      } else {
        ta.value = text;
        ta.dispatchEvent(new Event('input', { bubbles: true }));
      }
      setTimeout(function () {
        var btn = document.querySelector('button[data-testid="send-button"]')
          || document.querySelector('button[aria-label="Send prompt"]')
          || document.querySelector('button[aria-label*="Send"]');
        if (btn && !btn.disabled) { btn.click(); }
        else { ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true })); }
      }, 150);
    })();
  };

  // Once the composer exists the user is signed in — tell Rust so it parks this
  // window off-screen and auto-resends any pending turn.
  var readyTries = 0;
  (function checkReady() {
    if (window.__aceReady) return;
    var ta = document.getElementById('prompt-textarea')
      || document.querySelector('div[contenteditable="true"]')
      || document.querySelector('textarea');
    if (ta) { window.__aceReady = true; sig('ready', 'x', {}); return; }
    if (readyTries++ < 300) { setTimeout(checkReady, 300); }
  })();
})();
"#;

/// Per-request accumulation state for the webview relay: the last cumulative text
/// we emitted a delta from, the resolved conversation id, and whether the turn
/// already finished (so re-asserted done/late signals are ignored).
#[derive(Default)]
struct RelayEntry {
    full_text: String,
    conv_id: Option<String>,
    done: bool,
}

#[derive(Default)]
pub struct OpenAiRelay(Mutex<HashMap<String, RelayEntry>>);

/// Handles a signal from the injected script, delivered as an intercepted
/// `https://ace.relay/<kind>/<id>/<url-safe-base64-json>` navigation. Emits the
/// usual chat events so the frontend can't tell it apart from a normal stream.
fn handle_ace_relay(app: &tauri::AppHandle, rest: &str) {
    let mut parts = rest.splitn(3, '/');
    let kind = parts.next().unwrap_or("");
    let id = parts.next().unwrap_or("").to_string();
    let payload_b64 = parts.next().unwrap_or("");

    if kind == "ready" {
        // Signed in — park the window off-screen and let the UI auto-resend.
        if let Some(w) = app.get_webview_window("chatgpt-login") {
            let _ = w.set_skip_taskbar(true);
            let _ = w.set_position(tauri::PhysicalPosition::new(-4000, -4000));
        }
        let _ = app.emit("chatgpt-session://ready", ());
        return;
    }

    let json = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    if kind == "debug" {
        // Opt-in raw ChatGPT SSE capture, to study canvas/file event shapes.
        let raw = json.get("raw").and_then(|v| v.as_str()).unwrap_or("");
        let path = std::env::temp_dir().join("ace_openai_stream.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let _ = writeln!(f, "===== new ChatGPT response =====\n{raw}");
        }
        return;
    }

    let text = json.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let conv = json.get("conv").and_then(|v| v.as_str()).map(String::from);

    let state = app.state::<OpenAiRelay>();
    match kind {
        "chunk" | "done" => {
            // The page sends cumulative text; emit only the newly-appended part.
            // Collect what to emit under the lock, then emit after releasing it.
            let mut delta = String::new();
            let mut conv_id = None;
            let mut fire_done = false;
            if let Ok(mut map) = state.0.lock() {
                let e = map.entry(id.clone()).or_default();
                if e.done {
                    return; // turn already finished — ignore re-asserted/late signals
                }
                if e.conv_id.is_none() {
                    e.conv_id = conv.clone();
                }
                if text.starts_with(e.full_text.as_str()) {
                    delta = text[e.full_text.len()..].to_string();
                } else {
                    delta = text.clone();
                }
                e.full_text = text.clone();
                if kind == "done" {
                    e.done = true;
                    conv_id = e.conv_id.clone();
                    fire_done = true;
                }
            }
            if !delta.is_empty() {
                let _ = app.emit("chat://chunk", ChunkEvent { request_id: id.clone(), delta });
            }
            if fire_done {
                if let Some(cid) = conv_id {
                    let _ = app.emit(
                        "chat://web-saved",
                        WebSavedEvent { request_id: id.clone(), conversation_id: cid },
                    );
                }
                let _ = app.emit("chat://done", DoneEvent { request_id: id });
            }
        }
        "files" => {
            // Reconstruct sandbox files from the code-interpreter code.
            let code = json.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let names: Vec<String> = json
                .get("names")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let files = reconstruct_python_files(code, &names);
            if !files.is_empty() {
                let _ = app.emit("chat://files", FilesEvent { request_id: id, files });
            }
        }
        _ => {
            if let Ok(mut map) = state.0.lock() {
                let e = map.entry(id.clone()).or_default();
                if e.done {
                    return;
                }
                e.done = true;
            }
            let error = if text.is_empty() { "ChatGPT webview send failed".to_string() } else { text };
            let _ = app.emit("chat://error", ChatErrorEvent { request_id: id, error });
        }
    }
}

/// Drives the chatgpt.com webview to send `prompt` through the real composer, so
/// OpenAI's own code produces the sentinel tokens. Streams back via `openai_relay`.
#[command]
pub async fn openai_webview_send(
    app: tauri::AppHandle,
    request_id: String,
    prompt: String,
) -> Result<(), String> {
    let Some(win) = app.get_webview_window("chatgpt-login") else {
        // "CAPTCHA" marker makes the UI show the Verify/open button.
        return Err("Open ChatGPT first: click “Verify ChatGPT”, sign in, then send again. (CAPTCHA)".to_string());
    };
    // Drive it while hidden — the user only sees Ace, not the ChatGPT window.
    let id_js = serde_json::to_string(&request_id).map_err(|e| e.to_string())?;
    let text_js = serde_json::to_string(&prompt).map_err(|e| e.to_string())?;
    // The page may still be loading (so `__aceSend` isn't defined yet). Retry for
    // a few seconds, then relay a clear error instead of hanging on the dots.
    let script = format!(
        r#"(function(){{
            var id={id_js}, text={text_js}, tries=0;
            (function go(){{
              if (window.__aceSend) {{ window.__aceSend(id, text); return; }}
              if (tries++ < 40) {{ setTimeout(go, 150); return; }}
              try {{ window.__TAURI_INTERNALS__.invoke('openai_relay', {{ requestId: id, kind: 'error', data: 'ChatGPT page never finished loading — try again.' }}); }} catch (e) {{}}
            }})();
        }})();"#
    );
    win.eval(&script)
        .map_err(|e| format!("failed to drive ChatGPT webview: {e}"))?;
    Ok(())
}

/// Resolves a usable chatgpt.com browser session: a fresh read from the webview
/// if it's open, else the last cached capture. None if the user never verified.
async fn find_chatgpt_session(app: &tauri::AppHandle) -> Option<String> {
    let app_for_read = app.clone();
    if let Ok(Some(session)) =
        tauri::async_runtime::spawn_blocking(move || read_chatgpt_login_webview(&app_for_read)).await
    {
        if let Ok(mut guard) = app.state::<ChatGptWebSession>().0.lock() {
            *guard = Some(session.clone());
        }
        return Some(session.cookie_header);
    }
    if let Ok(guard) = app.state::<ChatGptWebSession>().0.lock() {
        if let Some(session) = guard.clone() {
            return Some(session.cookie_header);
        }
    }
    None
}

/// Fetches a chat-requirements token and, if PoW is demanded, solves it.
/// Returns `(requirements_token, Option<proof_token>)`.
async fn openai_chat_requirements(
    access_token: &str,
    account_id: &str,
    cookie: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let client = short_timeout_client()?;
    // A "pre" proof the requirements call expects; contents are unchecked.
    let pre = format!("gAAAAAC{}", openai_pow_config(0));
    let response = openai_common_headers(
        client.post("https://chatgpt.com/backend-api/sentinel/chat-requirements"),
        access_token,
        account_id,
        cookie,
    )
    .header("Content-Type", "application/json")
    .json(&serde_json::json!({ "p": pre }))
    .send()
    .await
    .map_err(|e| format!("chat-requirements request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        eprintln!("openai chat-requirements failed: HTTP {status}");
        if status.as_u16() == 401 {
            return Err("ChatGPT session expired — reconnect OpenAI.".to_string());
        }
        return Err(format!("ChatGPT requirements returned {status}"));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse chat-requirements: {e}"))?;

    let token = parsed
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "ChatGPT requirements returned no token".to_string())?
        .to_string();

    let proof = parsed.get("proofofwork").and_then(|pw| {
        let required = pw.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
        if !required {
            return None;
        }
        let seed = pw.get("seed").and_then(|v| v.as_str()).unwrap_or("");
        let difficulty = pw.get("difficulty").and_then(|v| v.as_str()).unwrap_or("");
        Some(openai_solve_pow(seed, difficulty))
    });

    // Turnstile (a browser CAPTCHA) can't be solved headlessly — fail clearly.
    if parsed
        .get("turnstile")
        .and_then(|t| t.get("required"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("ChatGPT wants you to verify (CAPTCHA) before saving to your account. Click “Verify ChatGPT”, clear the check in the window that opens, then send again.".to_string());
    }

    Ok((token, proof))
}

/// The conversation's current leaf node id, to parent the new turn onto. For a
/// new conversation there's nothing to fetch, so a fresh client uuid is the root.
async fn openai_leaf_id(
    access_token: &str,
    account_id: &str,
    conv_id: &str,
    cookie: Option<&str>,
) -> Option<String> {
    let client = short_timeout_client().ok()?;
    let response = openai_common_headers(
        client.get(format!("https://chatgpt.com/backend-api/conversation/{conv_id}")),
        access_token,
        account_id,
        cookie,
    )
    .send()
    .await
    .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let parsed: serde_json::Value = response.json().await.ok()?;
    parsed.get("current_node").and_then(|v| v.as_str()).map(String::from)
}

/// Streams ChatGPT's `/conversation` SSE, emitting `chat://chunk` text deltas and
/// returning the resolved conversation id (needed when a new one was created).
async fn stream_openai_web(
    app: &tauri::AppHandle,
    request_id: &str,
    access_token: &str,
    account_id: &str,
    conversation_id: Option<&str>,
    parent: &str,
    prompt: &str,
    requirements_token: &str,
    proof_token: Option<&str>,
    cookie: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Option<String>, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let mut body = serde_json::json!({
        "action": "next",
        "messages": [{
            "id": new_uuid_v4(),
            "author": { "role": "user" },
            "create_time": now,
            "content": { "content_type": "text", "parts": [prompt] },
            "metadata": {},
        }],
        "parent_message_id": parent,
        // ChatGPT's own model selection; Ace's codex model ids aren't web slugs.
        "model": "auto",
        "timezone_offset_min": 0,
        "history_and_training_disabled": false,
        "conversation_mode": { "kind": "primary_assistant" },
        "websocket_request_id": new_uuid_v4(),
    });
    if let Some(id) = conversation_id {
        body["conversation_id"] = serde_json::json!(id);
    }

    let client = streaming_client()?;
    let mut req = openai_common_headers(
        client.post("https://chatgpt.com/backend-api/conversation"),
        access_token,
        account_id,
        cookie,
    )
    .header("Content-Type", "application/json")
    .header("Accept", "text/event-stream")
    .header("OpenAI-Sentinel-Chat-Requirements-Token", requirements_token);
    if let Some(proof) = proof_token {
        req = req.header("Openai-Sentinel-Proof-Token", proof);
    }

    let response = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ChatGPT conversation request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        eprintln!("openai conversation failed: HTTP {status} — {text}");
        if status.as_u16() == 401 {
            return Err("ChatGPT session expired — reconnect OpenAI.".to_string());
        }
        return Err(format!("ChatGPT returned {status}"));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full_text = String::new();
    let mut conv_id = conversation_id.map(String::from);

    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => break,
            next = stream.next() => match next {
                Some(c) => c.map_err(|e| format!("stream read failed: {e}"))?,
                None => break,
            },
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].to_string();
            buffer.drain(..pos + 1);

            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else { continue };

            openai_apply_event(&parsed, &mut conv_id, &mut full_text, app, request_id);
        }
    }

    Ok(conv_id)
}

/// Interprets one ChatGPT SSE frame, emitting any newly-appended assistant text.
/// Handles both full message snapshots and the `v/o/p` delta protocol.
fn openai_apply_event(
    parsed: &serde_json::Value,
    conv_id: &mut Option<String>,
    full_text: &mut String,
    app: &tauri::AppHandle,
    request_id: &str,
) {
    if let Some(id) = parsed.get("conversation_id").and_then(|v| v.as_str()) {
        if conv_id.is_none() {
            *conv_id = Some(id.to_string());
        }
    }

    // Full snapshot: an assistant message with content.parts.
    if let Some(msg) = parsed.get("message") {
        if msg.pointer("/author/role").and_then(|v| v.as_str()) == Some("assistant") {
            let text = msg
                .pointer("/content/parts")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|p| p.as_str()).collect::<Vec<_>>().join(""))
                .unwrap_or_default();
            emit_openai_diff(&text, full_text, app, request_id);
            return;
        }
    }

    // Delta protocol: `v` carries either an initial object or appended text.
    if let Some(v) = parsed.get("v") {
        if let Some(s) = v.as_str() {
            // Plain appended text (o == "append" or the implicit default).
            if !s.is_empty() {
                *full_text += s;
                let _ = app.emit(
                    "chat://chunk",
                    ChunkEvent { request_id: request_id.to_string(), delta: s.to_string() },
                );
            }
        } else if v.is_object() {
            // Initial frame nests the whole message + conversation_id under `v`.
            openai_apply_event(v, conv_id, full_text, app, request_id);
        }
    }
}

/// Emits only the suffix of `text` not already streamed (snapshots resend the
/// whole message each frame).
fn emit_openai_diff(text: &str, full_text: &mut String, app: &tauri::AppHandle, request_id: &str) {
    if text.len() > full_text.len() && text.starts_with(full_text.as_str()) {
        let delta = text[full_text.len()..].to_string();
        *full_text = text.to_string();
        if !delta.is_empty() {
            let _ = app.emit(
                "chat://chunk",
                ChunkEvent { request_id: request_id.to_string(), delta },
            );
        }
    }
}

/// EXPERIMENTAL: send a turn into the user's real ChatGPT account conversation.
#[command]
pub async fn send_openai_web_message(
    app: tauri::AppHandle,
    request_id: String,
    conversation_id: Option<String>,
    prompt: String,
) -> Result<(), String> {
    let token = load_valid_token("openai")
        .await
        .ok_or_else(|| "Connect OpenAI first.".to_string())?;
    let account_id = token
        .id_token
        .as_deref()
        .and_then(chatgpt_account_id)
        .ok_or_else(|| "could not resolve ChatGPT account id".to_string())?;
    let access_token = token.access_token.clone();

    // A browser session (cf_clearance) from the verification webview, if the user
    // has completed it — this is what avoids the Turnstile CAPTCHA.
    let cookie = find_chatgpt_session(&app).await;

    let (requirements_token, proof_token) =
        openai_chat_requirements(&access_token, &account_id, cookie.as_deref()).await?;

    let conv = conversation_id.filter(|s| !s.is_empty());
    // Existing conversation → parent onto its current leaf; new → a client root.
    let parent = match &conv {
        Some(id) => openai_leaf_id(&access_token, &account_id, id, cookie.as_deref())
            .await
            .unwrap_or_else(new_uuid_v4),
        None => new_uuid_v4(),
    };

    let cancel = CancellationToken::new();
    if let Ok(mut map) = app.state::<ChatCancels>().0.lock() {
        map.insert(request_id.clone(), cancel.clone());
    }

    tauri::async_runtime::spawn(async move {
        let result = stream_openai_web(
            &app,
            &request_id,
            &access_token,
            &account_id,
            conv.as_deref(),
            &parent,
            &prompt,
            &requirements_token,
            proof_token.as_deref(),
            cookie.as_deref(),
            &cancel,
        )
        .await;

        if let Ok(mut map) = app.state::<ChatCancels>().0.lock() {
            map.remove(&request_id);
        }

        match result {
            Ok(resolved) => {
                if let Some(id) = resolved {
                    let _ = app.emit(
                        "chat://web-saved",
                        WebSavedEvent { request_id: request_id.clone(), conversation_id: id },
                    );
                }
                let _ = app.emit("chat://done", DoneEvent { request_id: request_id.clone() });
            }
            Err(e) => {
                eprintln!("ChatGPT write-back failed: {e}");
                let _ = app.emit(
                    "chat://error",
                    ChatErrorEvent { request_id: request_id.clone(), error: e },
                );
            }
        }
    });

    Ok(())
}

#[command]
pub async fn send_chat_message(
    app: tauri::AppHandle,
    provider: String,
    request_id: String,
    model: Option<String>,
    messages: Vec<ChatMessage>,
) -> Result<(), String> {
    let token = load_valid_token(&provider)
        .await
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
                    Some("response.completed") | Some("response.done") => {
                        let usage = parsed.pointer("/response/usage");
                        let input_tokens = usage
                            .and_then(|u| u.get("input_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let output_tokens = usage
                            .and_then(|u| u.get("output_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let used_model = parsed
                            .pointer("/response/model")
                            .and_then(|v| v.as_str())
                            .unwrap_or(model.unwrap_or("gpt-5.5"))
                            .to_string();
                        let _ = app.emit(
                            "chat://usage",
                            UsageEvent {
                                request_id: request_id.to_string(),
                                input_tokens,
                                output_tokens,
                                model: used_model,
                            },
                        );
                        return Ok(());
                    }
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
        "model": model.unwrap_or("claude-sonnet-4-5"),
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
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut used_model = model.unwrap_or("claude-sonnet-4-5").to_string();

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
                    Some("message_start") => {
                        if let Some(n) = parsed.pointer("/message/usage/input_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = n;
                        }
                        if let Some(m) = parsed.pointer("/message/model").and_then(|v| v.as_str()) {
                            used_model = m.to_string();
                        }
                    }
                    Some("message_delta") => {
                        // Cumulative output token count.
                        if let Some(n) = parsed.pointer("/usage/output_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = n;
                        }
                    }
                    Some("content_block_delta") => {
                        if let Some(delta) = parsed.pointer("/delta/text").and_then(|v| v.as_str()) {
                            let _ = app.emit(
                                "chat://chunk",
                                ChunkEvent { request_id: request_id.to_string(), delta: delta.to_string() },
                            );
                        }
                    }
                    Some("message_stop") => {
                        let _ = app.emit(
                            "chat://usage",
                            UsageEvent {
                                request_id: request_id.to_string(),
                                input_tokens,
                                output_tokens,
                                model: used_model.clone(),
                            },
                        );
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
