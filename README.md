# Codex Air

Codex Air is a focused native Windows workspace for Codex. It provides persistent multi-root projects and a real local connection to the official Codex App Server. It contains no simulated agent, cloud backend, or private authentication implementation.

## Status

The Windows preview provides saved multi-root workspaces and real Codex conversations. The chat includes a multiline composer, model and effort selection, image/file context, streamed Markdown, expandable tool output, approvals, questions, stop control, and session recovery. Codex Settings reads the installed harness's configuration, account usage, MCP servers, hooks, and plugins; app preferences are separate.

There is currently one active conversation per workspace and one running turn across the application. Multiple-session navigation, queue/steer, richer diff review, and sustained-session performance remain on the roadmap.

## Build on Windows

Requirements:

- Windows 11 x64
- Rust 1.98.1 (the repository toolchain)
- Visual Studio 2022 C++ Build Tools with the Windows 11 SDK
- `gpui-kit` 0.6.1 (the kit re-exports the compatible `gpui-pre` API; the resolved version in `Cargo.lock` is authoritative)

The locked native GPUI stack uses `gpui-pre` 0.3.4 with the Windows DirectX 11 and DirectWrite platform path.

Run the repository scripts:

```powershell
./scripts/dev.ps1
./scripts/build.ps1
```

The release executable is `target/release/codex-air.exe`. Its UI assets are embedded; the official Codex harness must also be installed.

Keyboard shortcuts include Ctrl+O to open a folder, Ctrl+Shift+O to add a folder, Ctrl+P to focus workspace search, Ctrl+, for Preferences, and Escape to clear search and return focus to the shell.

## Local state

State is written to `%LOCALAPPDATA%\Codex Air\state.json`. Set `CODEX_AIR_DATA_DIR` to override the data directory, which is useful for development and portable setups.

## Codex connection

Codex Air launches the newest available official local harness from the Codex desktop installation, PATH, or the Codex VS Code extension. `CODEX_AIR_CODEX_PATH` explicitly overrides discovery. The app performs the documented JSONL initialization sequence and reads the managed account state. If the local harness is already signed in, Codex Air discovers that session on launch. Codex Settings → Account exposes supported ChatGPT browser sign-in when needed. Codex Air never reads, copies, or stores access tokens.

This is separate from the developer-platform Agents API, which has its own API credentials and billing path. The project follows official documentation and does not reverse engineer private authentication endpoints.

See [the architecture](docs/architecture.md) and [the roadmap](docs/roadmap.md).
Measured startup and release verification results are recorded in [docs/verification.md](docs/verification.md).

## License

Codex Air is licensed under the GNU General Public License, version 3 only. See [LICENSE](LICENSE). Third-party dependencies remain under their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
