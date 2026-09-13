# Changelog

Codex Air follows Semantic Versioning.

## [0.3.0] - 2026-09-13

### Added
- Native multiline conversation composer with model and reasoning-effort selectors sourced from the installed App Server.
- Local image input, file-path context, stop control, session usage, and durable transcript recovery.
- Markdown messages, expandable command output and file diffs, queued approvals, and native answers to Codex questions.
- Codex Settings pages backed by real configuration, account, usage, MCP, hook, and plugin APIs; separate persisted app preferences.
- Background GitHub update discovery with actual results and errors.

### Changed
- Refined native surfaces, typography, and composer layout around the Codex desktop references.
- Separated user-facing release history from the engineering changelog.
- Keep menus to File and Edit, with About, release notes, and updates in the square Air menu.
- Select the newest installed official harness, including VS Code's bundled Windows executable.

### Fixed
- Embedded a scalable adaptation of the supplied Codex logo for the Settings button.
- Removed invalid App Server enum overrides that prevented prompts from starting.
- Accumulate streamed items by ID instead of discarding response fragments.
- Scope conversation output and drafts to their workspace; preserve Recent order on selection and history loading.
- Restore archives without stale toggles and keep archived workspaces out of the active sidebar.
- Persist state through File → Exit and retain maximization when activating an existing instance.
- Respond to unsupported server interactions rather than leaving turns silently blocked.


## 0.2.0 — 2026-09-13

- Added a native client header, application menu, release notes, and update entry point.
- Added Windows startup placement restoration, including maximized launches.
- Added managed local Codex account discovery and ChatGPT sign-in through the official App Server.
- Added project workspace management, Recent workspaces, pinning, archives, sidebar context actions, and File-menu recents.
- Added the initial task composer and thread-ID storage; protocol and transcript defects are corrected in 0.3.0.

## 0.1.0 — 2026-09-13

- Initial Windows-native Codex Air shell with durable workspace state.
