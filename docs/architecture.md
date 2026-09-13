# Architecture

Codex Air is a Windows-first Rust application built around a native GPUI shell. The local workspace boundary and App Server connection are separate so that projects remain stable while transport capabilities grow.

## Boundaries

- **Domain:** workspace roots, their persistent identity, the default working root, and future session-facing concepts.
- **UI:** native GPUI presentation and interaction. UI state is expressed through application-facing domain values rather than transport types.
- **Storage:** JSON state under `%LOCALAPPDATA%\Codex Air\state.json`, with `CODEX_AIR_DATA_DIR` as an override. Writes preserve the multi-root workspace and default root across launches.
- **Controller:** one worker owns command ordering and the in-memory `AppState`; it is the only writer to persisted workspace state.
- **App Server:** a dedicated background worker owns the local `codex app-server` process and JSONL protocol. It reports account state and login URLs to the UI but never exposes or persists credentials.
- **Windows shell:** process startup, native window integration, single-instance enforcement, and Windows-specific paths belong at the platform edge.

The shell owns no simulated agent. Sessions will receive backend-owned IDs when a real backend exists; the UI should not invent protocol identifiers or assume that local workspace identity is a remote session identity.

## Connection boundaries

The local ChatGPT subscription connection uses the Codex App Server's managed authentication. On startup Codex Air initializes the server and reads `account/read`; an existing local Codex login is reused automatically. If no account exists, Preferences requests the documented `account/login/start` ChatGPT flow and opens only the returned authorization URL. The App Server owns token persistence, refresh, and the localhost callback. The developer-platform Agents API is a separate execution path with separately authenticated and billed API credentials.

The App Server and Agents API architecture are documented by OpenAI:

- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Agents API architecture](https://developers.openai.com/api/docs/guides/agents-api/architecture)

No cloud backend is part of this milestone. The current connection stops at account discovery and managed sign-in; threads, turns, streamed activity, approvals, and diffs remain future App Server work. Future connection work must use published interfaces and official authentication guidance; private endpoint reverse engineering is out of scope.

## Dependency baseline

The application uses Rust 1.98.1 on Windows 11 x64, with `gpui-kit` pinned to 0.6.1. The kit re-exports the compatible `gpui-pre` API; the resolved `gpui-pre` 0.3.4 version in `Cargo.lock` is authoritative. The locked Windows rendering path uses DirectX 11 and DirectWrite. Atomic state replacement protects writes to the local JSON file. See [verification](verification.md) for measured checks.
