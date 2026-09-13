# Native shell verification

Measured on 2026-09-13 with the release MSVC build, Windows 11 Pro build 26200,
Intel Core i5-10400F and NVIDIA RTX 2060. This is a development-machine baseline,
not a clean-machine distribution certification.

## Build

`cargo fmt --check`, `cargo clippy --locked -- -D warnings`, and
`cargo build --release --locked` pass. The executable is 23,822,848 bytes
(22.7 MiB). No new test files or test-only application interfaces were added.

## Startup and idle behavior

Ten sequential warm launches restored one saved workspace. An external
PowerShell stopwatch began before `Start-Process`; Windows UI Automation was
polled every 15 ms until the saved workspace button existed and was enabled.
Each process was closed normally before the next launch. This measures an
accessible workspace, not a guarantee of GPU presentation or keyboard latency.

| Measurement | Median | p95 (nearest rank) |
| --- | ---: | ---: |
| Process launch to accessible restored workspace | 509.6 ms | 681.9 ms |
| Internal main-entry to first-frame callback | 449.2 ms | 555.6 ms |
| Internal main-entry to GPUI platform construction | 350.9 ms | 418.4 ms |

External samples in milliseconds: 681.92, 499.44, 474.10, 556.62, 505.41,
551.86, 513.70, 491.94, 459.59, 600.29. The first observed release launch earlier
in development reached its first-frame callback in about 1.3 seconds; caches
were not controlled, so it is not a formal cold-start measurement.

The proposed warm-start targets of median <=300 ms and p95 <=500 ms are **not
met**. The dominant measured phase is upstream GPUI Windows platform creation;
component and theme initialization take only a few milliseconds. Profile that
phase and window creation before adding speculative application caches.

One 10-second idle sample recorded 187.5 ms process CPU time, 64.2 MiB working
set and 82.2 MiB private memory. CPU time is summed across threads (1.875% of
one core for this sample). This is not a long-session or agent-load result.

For local startup tracing, set `CODEX_AIR_STARTUP_LOG` to an absolute JSONL file
path with an existing parent directory before launch. It is opt-in and records
only PID and timing phases. `first_frame_callback` is an event-loop callback,
not a presentation timestamp. Use `CODEX_AIR_DATA_DIR` to isolate runtime state.

## Native checks completed

- Open a real folder through the Windows folder picker; persist and reopen it.
- Add a second root, select it as default, and remove it. Workspace identity
  survives the 1 -> 2 -> 1 transition; the remaining root becomes default.
- Rename through the native client dialog and persist the new name.
- Switch workspaces and remove a recent workspace without touching its files.
- Reject adding the same folder twice and show a warning.
- Keep a missing root visible; locate its replacement through the folder picker
  while retaining both workspace and root identities.
- Preserve malformed state bytes in a `.bak` file before starting clean.
- Open and close with the state file exclusively locked; leave its bytes intact.
- Exit a second process and retain the existing app instance. This check caught
  and fixed handling of `EnumWindows` returning FALSE after finding a window.
- Normal shutdown and relaunch preserve workspace state. Native resizing and
  maximization were exercised; the layout was visually inspected at 100% DPI.

Checks used isolated, ignored runtime data under `artifacts/`; they did not
modify project-folder contents. These are recorded observations, not an
automated regression suite.

## Remaining verification and limitations

- Ctrl+O, Ctrl+P, Ctrl+Shift+O and Escape are implemented and their GPUI focus
  wiring was reviewed. Global keyboard injection was inconclusive because the
  active desktop focus changed; confirm shortcuts and modal focus manually.
- Actual 150%/200% DPI and mixed-monitor transitions need manual verification.
  Synthetic DPI messages did not establish a reliable visual result.
- GPUI menu controls expose labels but lacked UI Automation invoke/expand
  patterns in this build. Mouse interaction worked; full keyboard and screen
  reader coverage is not certified.
- Installer/signing, clean-machine launch, network-folder stalls, crash/power
  loss, large workspace lists and sustained resource use remain unverified.
- Session threads, streaming, approvals and diffs are not implemented in this
  milestone. No agent backend is simulated. The current build does establish a
  real local App Server connection for managed-account discovery and supported
  ChatGPT browser sign-in.

## 0.3.0 verification — 2026-09-13

The earlier milestone limitations above are historical. In 0.3.0, a prompt sent
through the native composer returned `AIR_UI_OK` from the real App Server. The
saved conversation, including the user prompt and assistant reply, was visually
verified after restart. A separate native close/relaunch check confirmed the
window restored maximized (`IsZoomed = true`). One measured release launch reached
its first-frame callback in 486.7 ms; this is a callback timing, not a presentation
timestamp or a multi-sample benchmark.

The Settings button's SVG, left-aligned sidebar, rounded composer, restored chat,
and native Codex Settings dialog were visually inspected. Live `config/read`,
`model/list`, `account/rateLimits/read`, `mcpServerStatus/list`, `hooks/list`, and
`plugin/list` requests succeeded against the installed official VS Code harness.
Release compilation, Clippy with warnings denied, and whitespace checks passed.

Approvals, question responses, attachments, and interruption are implemented from
the installed schema but were not all exercised through native UI in this pass.
Mixed-DPI first-paint behavior, long-session performance, queue/steer, multiple
conversations per workspace, and richer diff review remain unverified or pending.
