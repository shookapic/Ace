# Ace

A minimal, stealth desktop chat overlay for **OpenAI** and **Anthropic (Claude)** — sign in with your own accounts, chat with streaming responses, attach files, dictate by voice, and browse your past conversations. Built with Tauri v2 + React + TypeScript.

<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="Ace icon" />
</p>

## Demo

<p align="center">
  <img src="docs/demo_ace.gif" width="640" alt="Ace demo" />
</p>

## Features

- **Native OAuth login** for OpenAI and Anthropic — uses your own subscription, no API keys or third-party servers.
- **Streaming chat** with a per-provider model picker.
- **Provider-reactive UI** — the accent color reflects who you're talking to (clay for Claude, green for OpenAI).
- **Attachments** — send images and files.
- **Voice input** — dictate a message; it's transcribed to text before you send (via OpenAI Whisper).
- **Conversation history** — browse and reopen your real past conversations from claude.ai and ChatGPT (see [How history works](#how-history-works)).
- **Stealth mode** — hide the window from screenshots and screen shares (Windows/macOS).
- **System tray** — hide to tray with a customizable global shortcut (default `Ctrl+Shift+A`).
- **Adjustable window opacity** for an unobtrusive overlay.
- **Auto-updates** — Ace checks GitHub Releases on launch and can download, verify (signed), and install new versions in-place (Settings → Updates, or the in-app banner).

## Platform support

| Feature | Windows | macOS | Linux |
| --- | :---: | :---: | :---: |
| Login, chat, models, attachments, voice, history, tray, shortcut | ✅ | ✅ | ✅ |
| Hide from screen capture | ✅ | ✅¹ | — ² |
| Window opacity | ✅ | ✅¹ | — ² |

¹ Implemented but **untested** — the macOS-only code can't be compiled on Windows, so the first macOS build should verify it.
² No reliable cross-compositor API exists on Linux (Wayland forbids capture-exclusion; X11 has no standard). The app runs; these two controls are no-ops.

Windows is the primary, fully-tested platform. Each release also ships Linux and macOS builds from CI (see [Releases](../../releases)); macOS is **unsigned** and the macOS window effects are implemented but not yet verified on real hardware.

## How history works

Anthropic's inference OAuth token can't read conversation history, and claude.ai's web API is behind Cloudflare. Ace works around this **using your own logged-in session, on your own machine:**

- **Any browser / OS:** click **Connect claude.ai** in the history panel. Ace opens a real claude.ai login window (a genuine webview, so Cloudflare is solved normally), then reads the session from its *own* webview — no cookie decryption.
- **Firefox fast-path (Windows/macOS):** if you're already signed into claude.ai in Firefox, Ace reads that session automatically for zero-click history.

OpenAI (ChatGPT) history uses your OAuth token directly and works on any setup.

## Install

Grab the latest build for your OS from [Releases](../../releases):

- **Windows** — `Ace_x.y.z_x64-setup.exe` (NSIS) or `Ace_x.y.z_x64_en-US.msi` (MSI). Unsigned, so SmartScreen warns on first run — click **More info → Run anyway**.
- **Linux** — `Ace_x.y.z_amd64.AppImage` (`chmod +x` then run), or the `.deb` / `.rpm` package.
- **macOS** — `Ace_x.y.z_universal.dmg` (runs on both Intel and Apple Silicon). **Unsigned** — right-click the app → **Open** on first launch, or run `xattr -cr /Applications/Ace.app`.

## Build from source

Prerequisites: [Node.js](https://nodejs.org) 18+, [Rust](https://rustup.rs), and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev      # run in development
npm run tauri build    # produce installers in src-tauri/target/release/bundle/
```

Build on the OS you're targeting — Tauri cannot cross-compile macOS/Linux from Windows. Distributing a macOS build additionally requires Apple Developer signing + notarization.

### Releasing updatable builds

Auto-update artifacts are signed with a minisign keypair. To cut a release that existing installs can update to:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/ace.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<key password>"
npm run tauri build
```

This emits the installers plus `*.sig` signatures. Publish a `latest.json` manifest (version, `pub_date`, and per-platform `signature` + download `url`) alongside the installer on the GitHub release; the app's updater endpoint reads `releases/latest/download/latest.json`. **Keep the private key secret — it is never committed.** Losing it means no existing install can auto-update.

## Tech stack

React · TypeScript · Tailwind CSS · Framer Motion · Zustand · Tauri v2 (Rust)

## Privacy

Ace talks directly to OpenAI and Anthropic with your own credentials, stored in your OS keychain. Nothing is sent to any third-party server. The Firefox fast-path reads your local `cookies.sqlite` only to reuse an existing claude.ai session on your own machine.

## License

[PolyForm Noncommercial License 1.0.0](LICENSE) — free to use, modify, and share for any **noncommercial** purpose. Commercial use is not permitted.
