# Third-party notices

Codex Air includes third-party dependencies distributed under their own licenses. Those licenses are separate from the GPL-3.0-only license covering Codex Air's original code. A dependency's license does not grant rights to relicense that dependency, and this file makes no guarantee that the complete project can be commercially relicensed.

The current dependency set is declared in [Cargo.toml](Cargo.toml). License notices and source terms for each dependency must be reviewed from the exact resolved Cargo dependency graph before a release artifact is distributed. In particular, review the resolved licenses for `gpui-kit`, its `gpui-pre` family, `anyhow`, `serde`, `serde_json`, `uuid`, `tempfile`, `async-channel`, `windows`, and `winresource`, including transitive dependencies.

When packaging a release, include the applicable upstream license texts and notices alongside the executable or in the distribution's notices directory. Keep third-party notices distinct from the project's contribution and relicensing policy.

## UI assets

The planned UI asset set includes these separately licensed assets:

- [Lucide](https://github.com/lucide-icons/lucide/blob/main/LICENSE) icons, licensed under the ISC License.
- [Catppuccin](https://github.com/catppuccin/catppuccin/blob/main/LICENSE) palette, licensed under the MIT License.

The upstream license files linked above are the source notices to preserve with any release that distributes those assets.
