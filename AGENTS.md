# Codex Air

Read the owner's canonical `~/.codex/agent-preferences/PREFERENCES.md` and apply
the project-working-style and signal-first-ui skills. This is an independent,
Windows-first Rust/GPUI client. Never copy codexRS source or UI.

Current milestone: native shell, real multi-root workspace management, local
state, release startup verification. No simulated agent or cloud support.

Use the project-scoped native Codex agents in `.codex/agents/` when work can
proceed independently. The parent assigns explicit file ownership and integrates:
- protocol-researcher: official protocol/authentication and dependency evidence;
- native-ui: Windows/GPUI presentation, focus and interaction implementation;
- runtime-reviewer: read-only correctness and native runtime verification.

Keep UI independent of transport types. Local Codex subscription authentication
and developer-platform API credentials are different execution paths. Research
current official docs and source before implementing either backend.

No new test files, test-only helpers, or fixtures without explicit owner approval.
Use direct runtime checks and Cargo checks. Do not use browser automation.
Do not commit, push, publish, or accept outside code contributions without the
appropriate task authorization and contribution policy.
