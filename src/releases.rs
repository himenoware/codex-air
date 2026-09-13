//! User-facing history, independent from the engineering changelog.
pub struct ReleaseNote {
    pub version: &'static str,
    pub date: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

pub const RELEASES: &[ReleaseNote] = &[
    ReleaseNote {
        version: "0.3.0",
        date: "2026-09-13",
        title: "A clearer place to work",
        body: "Send real Codex tasks from a rounded multiline composer, choose your model and effort, attach context, and read the full response with expandable tool activity. Saved conversations reopen with their history. Codex Settings now reads your harness's actual settings and usage; app preferences remain separate. This release also fixes the invalid-request error and discovers newer official harnesses installed with VS Code.",
    },
    ReleaseNote {
        version: "0.2.0",
        date: "2026-09-13",
        title: "Organize your workspaces",
        body: "Pin workspaces, open recent projects from File, and restore archived entries from Archives. This early preview includes local Codex account discovery and an initial prompt composer; the full settings and session experience is still incomplete.",
    },
    ReleaseNote {
        version: "0.1.0",
        date: "2026-09-13",
        title: "The first Windows preview",
        body: "Codex Air begins with a native window, saved multi-folder workspaces, local Codex sign-in discovery, and an initial App Server connection.",
    },
];
