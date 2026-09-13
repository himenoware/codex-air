# Roadmap

The roadmap follows the smallest useful native workflow first, then adds a real connection behind stable UI and storage boundaries.

## Milestone 1: native shell and workspace state

- Windows-native GPUI shell
- Real multi-root workspace management
- Add and remove roots, with a persistent default working root
- Local state at `%LOCALAPPDATA%\Codex Air\state.json`
- `CODEX_AIR_DATA_DIR` override
- Release startup verification

There is no agent simulation or cloud backend in this milestone. The shell starts the official local App Server to discover its managed account state and request supported ChatGPT sign-in when needed; threads and agent work are not implemented yet. The controller, atomic local-state writes, and Windows single-instance boundary are part of the native shell.

## Next milestones

1. Real App Server threads, task composition, and persisted session navigation.
2. Streamed activity, approvals, and interactions owned by the selected backend.
3. Context diffs and long-session behavior.
4. Interaction and release polish for daily Windows use.

The developer-platform Agents API remains a separate, future credential and billing path. It will not be treated as interchangeable with a ChatGPT subscription connection.
