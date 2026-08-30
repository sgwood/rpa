# AI Task Control

[![Verify and package](https://github.com/sgwood/rpa/actions/workflows/verify-and-package.yml/badge.svg)](https://github.com/sgwood/rpa/actions/workflows/verify-and-package.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

AI Task Control is a local-first, cross-platform desktop console for observing and continuing tasks across AI coding tools on macOS and Windows.

The project currently integrates with OpenAI Codex, Claude, Cursor, and, experimentally, Antigravity IDE. It uses official hooks and local command interfaces where available, normalizes provider events into one state model, and keeps its API bound to `127.0.0.1` by default.

> This is an early `0.1.x` project. A detected IDE process means the tool is connected; it does not prove that a task is running. Live task counts are based on observed hook events.

## Features

- A live overview of connected tools and active or waiting tasks
- Unified task state, timelines, event deduplication, and offline spooling
- Safe continuation through `SEND_NEXT`, managed `RESUME_AND_SEND`, or open-and-copy fallbacks
- Local SQLite storage with encrypted sensitive command bodies
- Redacted diagnostics and native credential storage
- Feishu notifications and completion summaries
- Tauri desktop packaging for macOS and Windows

## Development

Prerequisites: Rust 1.98, Node.js 24, npm, and macOS or Windows.

```bash
git clone https://github.com/sgwood/rpa.git
cd rpa
npm --prefix apps/desktop ci

# Terminal 1
cargo run -p ai-rpa-node -- serve

# Terminal 2
npm --prefix apps/desktop run dev
```

Run the complete local verification suite:

```bash
./scripts/verify.sh
```

On Windows PowerShell, run `.\scripts\verify.ps1` instead.

Build native installers with `npm --prefix apps/desktop run tauri build -- --bundles app,dmg` on macOS or `npm --prefix apps/desktop run tauri build -- --bundles nsis` on Windows. MSI builds require the deprecated Windows VBSCRIPT optional feature.

See [CONTRIBUTING.md](CONTRIBUTING.md) before submitting a change. Security issues must be reported privately according to [SECURITY.md](SECURITY.md).

## Project status

A four-tool delivery proof of concept and signed local application install have been verified on macOS. Windows Native/WSL, production Feishu webhooks, macOS notarization, and formal Windows signing still require release-candidate validation. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) and [TEST_PLAN.md](TEST_PLAN.md) for evidence boundaries.

Product names and trademarks belong to their respective owners. This independent open-source project is not affiliated with or endorsed by OpenAI, Anthropic, Cursor, or Antigravity IDE.

## License

Licensed under the [Apache License 2.0](LICENSE).
