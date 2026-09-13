# Roadmap

The roadmap follows the smallest useful native workflow first, then adds a real connection behind stable UI and storage boundaries.

## Milestone 1: native shell and workspace state

- Windows-native GPUI shell
- Real multi-root workspace management
- Add and remove roots, with a persistent default working root
- Local state at `%LOCALAPPDATA%\Codex Air\state.json`
- `CODEX_AIR_DATA_DIR` override
- Release startup verification

There is no simulated agent or cloud backend. The native shell starts the official
local App Server and uses its managed account. The initial thread composer shipped
in 0.1.0, but protocol values and streaming presentation required correction.
Version 0.3.0 addresses those defects and separates Codex Settings from app preferences.

## Next milestones

1. Real App Server threads, task composition, and persisted session navigation.
2. Streamed activity, approvals, and interactions owned by the selected backend.
3. Context diffs and long-session behavior.
4. Interaction and release polish for daily Windows use.

The developer-platform Agents API remains a separate, future credential and billing path. It will not be treated as interchangeable with a ChatGPT subscription connection.

## Requests that still require end-to-end verification

These are tracked requirements, not completed features:

- Multiple conversations per workspace, thread history loading, creation, and switching.
- File and image attachments with previews and removal.
- Composer model/effort selection, permission profile, context usage, and rate-limit status.
- Stop, queue, and steer during an active turn.
- Complete command/file approval and user-input flows; no unanswered server requests.
- Markdown/code rendering, selectable transcript, copy actions, and natural diff review.
- Settings parity with the supplied General, Configuration, Personalization, Usage,
  MCP, Hooks, Plugins, and Account references wherever the official runtime supports it.
- Memory controls and model-specific reasoning options must use real capabilities.
- Archive manager recovery, keyboard navigation, and File recents on the real Windows build.
- Startup at saved size/mode without a resize flash on multiple DPI settings.
- Real update discovery, a downloadable Windows release, and accurate historical release notes.

Never replace these requirements with static labels and report them as delivered.
