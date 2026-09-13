use crate::{
    app_server::{self, AccountStatus},
    codex_settings::{CodexSettings, SettingsChanged},
    composer_controls::{ComposerControls, StatusView},
    controller::{self, Command},
    platform,
    preferences::PreferencesView,
    questions::QuestionView,
    theme::{self, *},
    workspace::{AppState, WorkspaceRoot},
};
use gpui_kit::{
    assets::IconName,
    base::{Disableable, h_resizable, resizable_panel, text::TextView},
    component::{
        Icon, Root, Sizable, TitleBar, WindowExt,
        button::{Button, ButtonVariants},
        dialog::Confirm,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
        scroll::ScrollableElement,
    },
    prelude::FluentBuilder,
    *,
};
use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::Sender,
};
use uuid::Uuid;

actions!(
    air,
    [
        OpenFolder,
        SwitchWorkspace,
        AddFolder,
        RenameWorkspace,
        RemoveWorkspace,
        RefreshFolders,
        ClearSearch,
        Preferences,
        OpenCodexSettings,
        About,
        CheckUpdates,
        ReleaseNotes,
        Archives,
        Exit
    ]
);

#[derive(Clone)]
struct ActivityEntry {
    workspace: Uuid,
    id: Option<String>,
    kind: String,
    text: String,
    user: bool,
}

fn activity_label(kind: &str) -> &str {
    match kind {
        "commandExecution" => "Ran command",
        "fileChange" => "Edited files",
        "mcpToolCall" | "dynamicToolCall" => "Used tool",
        "webSearch" => "Searched the web",
        "collabAgentToolCall" => "Agent activity",
        "reasoning" => "Thinking",
        "status" => "Codex",
        _ => "Activity",
    }
}

pub struct Shell {
    state: AppState,
    availability: HashMap<Uuid, bool>,
    sender: Sender<Command>,
    search: Entity<InputState>,
    composer: Entity<TextareaState>,
    composer_controls: Entity<ComposerControls>,
    focus: FocusHandle,
    loaded: bool,
    warning: Option<String>,
    closing: bool,
    first_render: bool,
    can_save: bool,
    availability_generation: u64,
    account: AccountStatus,
    account_sender: Sender<app_server::Request>,
    turn_active: bool,
    active_turn_workspace: Option<Uuid>,
    loaded_threads: HashSet<(Uuid, String)>,
    token_usage: HashMap<Uuid, (String, i64, Option<i64>)>,
    attachments: Vec<std::path::PathBuf>,
    drafts: HashMap<Uuid, (String, Vec<std::path::PathBuf>)>,
    conversation_scroll: ScrollHandle,
    activity: Vec<ActivityEntry>,
    pending_approvals: Vec<app_server::Approval>,
    pending_questions: HashMap<Uuid, Entity<QuestionView>>,
    expanded_activity: HashSet<(Uuid, String)>,
    folders_collapsed: bool,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, updates) = controller::start();
        let (account_sender, account_updates) = app_server::start();
        let composer_controls =
            cx.new(|cx| ComposerControls::new(account_sender.clone(), None, cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a workspace…"));
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Codex to work on this workspace…")
                .auto_grow(2, 8)
                .submit_on_enter(true)
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let input_subscription = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let query = this.search.read(cx).value().to_lowercase();
                if let Some(workspace) = this.state.workspaces.iter().find(|w| {
                    w.display_name().to_lowercase().contains(&query)
                        || w.roots
                            .iter()
                            .any(|r| r.display_path().to_lowercase().contains(&query))
                }) {
                    this.send(Command::Select(workspace.id));
                    this.search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    window.focus(&this.focus, cx);
                }
            }
            cx.notify();
        });
        let bounds_subscription = cx.observe_window_bounds(window, |this, window, _| {
            if this.loaded
                && let Some(placement) = platform::placement(window)
            {
                this.send(Command::Placement(placement));
            }
        });
        let composer_subscription =
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { secondary, shift } = event
                    && !shift
                    && (this.state.preferences.enter_sends || *secondary)
                {
                    this.start_task(window, cx);
                }
            });
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                if !this.closing {
                    this.closing = true;
                    if let Some(placement) = platform::placement(window) {
                        this.send(Command::Placement(placement));
                    }
                    this.send(Command::Close);
                    cx.notify();
                }
            });
            false
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(snapshot) = updates.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        if snapshot.closed {
                            cx.quit();
                            return;
                        }
                        if snapshot.initial
                            && let Some(saved) = &snapshot.state.window
                            && !saved.maximized
                        {
                            platform::restore(window, saved);
                        }
                        if this.state.active_workspace != snapshot.state.active_workspace {
                            if let Some(previous) = this.state.active_workspace {
                                this.drafts.insert(
                                    previous,
                                    (
                                        this.composer.read(cx).value().to_string(),
                                        std::mem::take(&mut this.attachments),
                                    ),
                                );
                            }
                            let (text, attachments) = snapshot
                                .state
                                .active_workspace
                                .and_then(|id| this.drafts.remove(&id))
                                .unwrap_or_default();
                            this.attachments = attachments;
                            this.composer
                                .update(cx, |input, cx| input.set_value(text, window, cx));
                            this.conversation_scroll.scroll_to_bottom();
                        }
                        this.state = snapshot.state;
                        if let Some(workspace) = this.state.active()
                            && let Some(thread_id) = &workspace.thread_id
                            && this
                                .loaded_threads
                                .insert((workspace.id, thread_id.clone()))
                        {
                            let _ = this.account_sender.send(app_server::Request::ReadThread {
                                workspace: workspace.id,
                                thread_id: thread_id.clone(),
                            });
                        }
                        let cwd = this.state.active().and_then(|workspace| {
                            workspace
                                .default_root
                                .and_then(|id| workspace.roots.iter().find(|root| root.id == id))
                                .or_else(|| workspace.roots.first())
                                .map(|root| root.path.clone())
                        });
                        this.composer_controls.update(cx, |controls, cx| {
                            controls.set_workspace(cwd, cx);
                        });
                        let enter_sends = this.state.preferences.enter_sends;
                        this.composer.update(cx, |input, cx| {
                            input.set_submit_on_enter(enter_sends, cx);
                        });
                        this.check_folders(window, cx);
                        this.warning = snapshot.warning;
                        this.loaded = true;
                        if snapshot.close_failed {
                            this.closing = false;
                        }
                        this.can_save = snapshot.can_save;
                        let title = this
                            .state
                            .active()
                            .map(|w| format!("{} — Codex Air", w.display_name()))
                            .unwrap_or_else(|| "Codex Air".into());
                        window.set_window_title(&title);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(update) = account_updates.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        match update {
                            app_server::Update::Status(status) => this.account = status,
                            app_server::Update::LoginUrl(url) => cx.open_url(&url),
                            app_server::Update::ThreadStarted {
                                workspace,
                                thread_id,
                            } => {
                                this.loaded_threads.insert((workspace, thread_id.clone()));
                                this.send(Command::SetThread(workspace, thread_id));
                            }
                            app_server::Update::ThreadLoaded {
                                workspace,
                                thread_id,
                            } => {
                                this.loaded_threads.insert((workspace, thread_id));
                            }
                            app_server::Update::TokenUsage {
                                workspace,
                                thread_id,
                                used,
                                context_window,
                            } => {
                                this.token_usage
                                    .insert(workspace, (thread_id, used, context_window));
                            }
                            app_server::Update::Item {
                                workspace,
                                id,
                                kind,
                                text,
                            } => {
                                let follow = this.conversation_scroll.offset().y.abs()
                                    >= this.conversation_scroll.max_offset().y - px(80.);
                                this.upsert_item_activity(workspace, id, kind, text);
                                if follow && this.state.active_workspace == Some(workspace) {
                                    this.conversation_scroll.scroll_to_bottom();
                                }
                            }
                            app_server::Update::ApprovalRequested(approval) => {
                                this.pending_approvals.push(approval);
                            }
                            app_server::Update::QuestionsRequested {
                                workspace,
                                id,
                                questions,
                            } => {
                                let sender = this.account_sender.clone();
                                let view = cx
                                    .new(|cx| QuestionView::new(id, questions, sender, window, cx));
                                this.pending_questions.insert(workspace, view);
                            }
                            app_server::Update::TurnFinished { workspace, error } => {
                                this.turn_active = false;
                                this.active_turn_workspace = None;
                                this.pending_questions.remove(&workspace);
                                this.pending_approvals
                                    .retain(|approval| approval.workspace != workspace);
                                if let Some(error) = error {
                                    this.warning = Some(error);
                                }
                            }
                            app_server::Update::WorkspaceError { workspace, message } => {
                                if this.state.active_workspace == Some(workspace) {
                                    this.warning = Some(message);
                                }
                            }
                            app_server::Update::Error(error) => {
                                this.turn_active = false;
                                this.warning = Some(error);
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        window.on_next_frame(move |_, cx| {
            crate::diagnostics::mark("first_frame_callback");
            cx.background_spawn(async {
                crate::diagnostics::flush();
            })
            .detach();
        });
        Self {
            state: AppState::default(),
            availability: HashMap::new(),
            sender,
            search,
            composer,
            composer_controls,
            focus,
            loaded: false,
            warning: None,
            closing: false,
            first_render: true,
            can_save: true,
            availability_generation: 0,
            account: AccountStatus::default(),
            account_sender,
            turn_active: false,
            active_turn_workspace: None,
            loaded_threads: HashSet::new(),
            token_usage: HashMap::new(),
            attachments: Vec::new(),
            drafts: HashMap::new(),
            conversation_scroll: ScrollHandle::new(),
            activity: Vec::new(),
            pending_approvals: Vec::new(),
            pending_questions: HashMap::new(),
            expanded_activity: HashSet::new(),
            folders_collapsed: true,
            _subscriptions: vec![
                input_subscription,
                composer_subscription,
                bounds_subscription,
            ],
        }
    }

    fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }

    fn append_user_activity(&mut self, workspace: Uuid, text: String) {
        if text.is_empty() {
            return;
        }
        self.activity.push(ActivityEntry {
            workspace,
            id: None,
            kind: "user".into(),
            text,
            user: true,
        });
    }

    fn upsert_item_activity(&mut self, workspace: Uuid, id: String, kind: String, text: String) {
        let is_user = kind == "userMessage";
        if is_user
            && let Some(entry) = self.activity.iter_mut().rev().find(|entry| {
                entry.workspace == workspace
                    && entry.user
                    && entry.id.is_none()
                    && (entry.text == text
                        || text
                            .strip_prefix(&entry.text)
                            .is_some_and(|rest| rest.starts_with("\n\nAttached local file")))
            })
        {
            entry.id = Some(id);
            return;
        }
        if let Some(entry) = self
            .activity
            .iter_mut()
            .find(|entry| entry.workspace == workspace && entry.id.as_deref() == Some(id.as_str()))
        {
            entry.kind = kind;
            entry.text = text;
        } else {
            self.activity.push(ActivityEntry {
                workspace,
                id: Some(id),
                kind,
                text,
                user: is_user,
            });
        }
    }

    fn toggle_activity(&mut self, workspace: Uuid, id: String, cx: &mut Context<Self>) {
        let key = (workspace, id);
        if !self.expanded_activity.remove(&key) {
            self.expanded_activity.insert(key);
        }
        cx.notify();
    }

    fn resolve_approval(&mut self, accept: bool, cx: &mut Context<Self>) {
        let Some(index) = self
            .pending_approvals
            .iter()
            .position(|approval| Some(approval.workspace) == self.state.active_workspace)
        else {
            return;
        };
        let approval = self.pending_approvals.remove(index);
        let _ = self
            .account_sender
            .send(app_server::Request::ResolveApproval {
                id: approval.id,
                accept,
            });
        cx.notify();
    }

    fn start_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.turn_active || !self.account.connected {
            return;
        }
        let Some((workspace_id, cwd, thread_id)) = self.state.active().and_then(|workspace| {
            workspace
                .default_root
                .and_then(|id| workspace.roots.iter().find(|root| root.id == id))
                .or_else(|| workspace.roots.first())
                .map(|root| (workspace.id, root.path.clone(), workspace.thread_id.clone()))
        }) else {
            self.warning = Some("Open a workspace folder before starting a Codex task.".into());
            cx.notify();
            return;
        };
        let mut text = self.composer.read(cx).value().trim().to_owned();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        if text.is_empty() {
            text = "Please inspect the attached context.".into();
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.turn_active = true;
        self.active_turn_workspace = Some(workspace_id);
        self.warning = None;
        self.append_user_activity(workspace_id, text.clone());
        self.conversation_scroll.scroll_to_bottom();
        let (model, effort) = self.composer_controls.read(cx).selection();
        let _ = self.account_sender.send(app_server::Request::StartTurn {
            workspace: workspace_id,
            cwd,
            text,
            thread_id,
            model,
            effort,
            attachments: std::mem::take(&mut self.attachments),
        });
        cx.notify();
    }

    fn attach_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach context".into()),
        });
        cx.spawn_in(window, async move |this, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                let _ = this.update(cx, |this, cx| {
                    for path in paths {
                        if !this.attachments.contains(&path) {
                            this.attachments.push(path);
                        }
                    }
                    cx.notify();
                });
            }
            Ok(Ok(None)) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.warning = Some(format!("Could not open the file picker: {error:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn session_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let thread = self.state.active().and_then(|w| w.thread_id.clone());
        let usage = self
            .state
            .active_workspace
            .and_then(|id| self.token_usage.get(&id))
            .map(|(_, used, limit)| (*used, *limit));
        let sender = self.account_sender.clone();
        let status = cx.new(|cx| StatusView::new(sender, thread, usage, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Session status")
                .w(px(520.))
                .child(status.clone())
                .footer(dialog_footer("Close", false))
        });
    }

    fn check_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.availability_generation += 1;
        let generation = self.availability_generation;
        let roots = self
            .state
            .active()
            .map(|w| w.roots.clone())
            .unwrap_or_default();
        self.availability.clear();
        let check = cx.background_spawn(async move {
            roots
                .into_iter()
                .map(|root| (root.id, root.path.is_dir()))
                .collect::<HashMap<_, _>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let availability = check.await;
            let _ = this.update(cx, |this, cx| {
                if this.availability_generation == generation {
                    this.availability = availability;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn choose_folder(
        &mut self,
        target: Option<(Uuid, Option<Uuid>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_save {
            self.warning = Some("Workspace changes are disabled until the saved state can be read safely. Resolve the state file error and restart Codex Air.".into());
            cx.notify();
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                if target.is_some() {
                    "Select folder"
                } else {
                    "Open folder"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, _| {
                        this.send(match target {
                            None => Command::Open(path),
                            Some((workspace, None)) => Command::AddRoot(workspace, path),
                            Some((workspace, Some(root))) => {
                                Command::RelocateRoot(workspace, root, path)
                            }
                        })
                    });
                }
            }
            Ok(Ok(None)) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.warning = Some(format!("Could not open the folder picker: {error:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_folder(None, window, cx);
    }
    fn add_folder(&mut self, _: &AddFolder, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.state.active_workspace {
            self.choose_folder(Some((id, None)), window, cx);
        }
    }
    fn switch_workspace(
        &mut self,
        _: &SwitchWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.update(cx, |input, cx| input.focus(window, cx));
    }
    fn clear_search(&mut self, _: &ClearSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn refresh(&mut self, _: &RefreshFolders, _: &mut Window, _: &mut Context<Self>) {
        self.send(Command::Refresh);
    }

    fn preferences(&mut self, _: &Preferences, window: &mut Window, cx: &mut Context<Self>) {
        let settings =
            cx.new(|_| PreferencesView::new(self.state.preferences.clone(), self.sender.clone()));
        window.focus(&self.focus, cx);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Preferences")
                .w(px(560.))
                .child(settings.clone())
                .footer(dialog_footer("Close", false))
        });
    }

    fn codex_settings(
        &mut self,
        _: &OpenCodexSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self
            .state
            .active()
            .and_then(|workspace| {
                workspace
                    .default_root
                    .and_then(|id| workspace.roots.iter().find(|root| root.id == id))
                    .or_else(|| workspace.roots.first())
            })
            .map(|root| root.path.clone());
        let settings = cx.new(|cx| {
            CodexSettings::new(
                self.account_sender.clone(),
                cwd,
                self.account.clone(),
                window,
                cx,
            )
        });
        self._subscriptions.push(cx.subscribe_in(
            &settings,
            window,
            |this, _, _: &SettingsChanged, _, cx| {
                this.composer_controls
                    .update(cx, |controls, cx| controls.refresh(cx));
            },
        ));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Codex Settings")
                .w(px(820.))
                .child(settings.clone())
                .footer(dialog_footer("Close", false))
        });
    }

    fn exit(&mut self, _: &Exit, window: &mut Window, cx: &mut Context<Self>) {
        if !self.closing {
            self.closing = true;
            if let Some(placement) = platform::placement(window) {
                self.send(Command::Placement(placement));
            }
            self.send(Command::Close);
            cx.notify();
        }
    }

    fn about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, |dialog, _, _| {
            dialog
                .title("About Codex Air")
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Codex Air"))
                        .child(theme::caption(format!(
                            "Version {}",
                            env!("CARGO_PKG_VERSION")
                        )))
                        .child(theme::caption("A native Windows workspace for Codex.")),
                )
                .footer(dialog_footer("Close", false))
        });
    }

    fn check_updates(&mut self, _: &CheckUpdates, window: &mut Window, cx: &mut Context<Self>) {
        let status = cx.new(crate::updates::UpdateView::new);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Check for updates")
                .child(status.clone())
                .child(theme::caption(format!(
                    "Version {}",
                    env!("CARGO_PKG_VERSION")
                )))
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-updates")
                                .label("Close")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("open-releases")
                                .primary()
                                .label("Open releases")
                                .on_click(|_, _, cx| {
                                    cx.open_url("https://github.com/himenoware/codex-air/releases");
                                }),
                        ),
                )
        });
    }

    fn release_notes(&mut self, _: &ReleaseNotes, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, |dialog, _, _| {
            let mut notes = div()
                .id("release-history")
                .max_h(px(420.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_5();
            for note in crate::releases::RELEASES {
                notes = notes.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(theme::caption(format!("{} · {}", note.version, note.date)))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(note.title))
                        .child(note.body),
                );
            }
            dialog
                .title("Release notes")
                .child(notes)
                .footer(dialog_footer("Close", false))
        });
    }

    fn archives(&mut self, _: &Archives, window: &mut Window, cx: &mut Context<Self>) {
        let archived: Vec<_> = self
            .state
            .workspaces
            .iter()
            .filter(|workspace| workspace.archived)
            .map(|workspace| (workspace.id, workspace.display_name()))
            .collect();
        let sender = self.sender.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut content = div().flex().flex_col().gap_2();
            if archived.is_empty() {
                content = content.child(theme::caption("No archived workspaces."));
            } else {
                for (id, name) in &archived {
                    let sender = sender.clone();
                    let id = *id;
                    content = content.child(
                        Button::new(SharedString::from(format!("restore-archive-{id}")))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .label(format!("Restore  {name}"))
                            .on_click(move |_, window, cx| {
                                let _ = sender.send(Command::ToggleArchive(id));
                                window.close_dialog(cx);
                            }),
                    );
                }
            }
            dialog
                .title("Archives")
                .child(content)
                .footer(dialog_footer("Close", false))
        });
    }

    fn app_header(&self, _: &Context<Self>) -> AnyElement {
        let workspace = self
            .state
            .active()
            .map(|workspace| workspace.display_name())
            .unwrap_or_else(|| "No workspace open".into());
        TitleBar::new()
            .bg(rgb(SIDEBAR))
            .border_color(rgb(BORDER))
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .child(
                        Button::new("app-menu")
                            .ghost()
                            .small()
                            .w(px(34.))
                            .h_full()
                            .text_color(rgb(ACCENT))
                            .icon(Icon::default().path("icons/air.svg"))
                            .accessibility_label("Codex Air menu")
                            .tooltip("Codex Air")
                            .dropdown_menu(|menu, _, _| {
                                menu.menu("About Codex Air", Box::new(About))
                                    .menu("Release notes", Box::new(ReleaseNotes))
                                    .separator()
                                    .menu("Check for updates…", Box::new(CheckUpdates))
                            }),
                    )
                    .child(self.menubar())
                    .child(div().h(px(18.)).border_l_1().border_color(rgb(BORDER)))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(TEXT))
                            .child(workspace),
                    ),
            )
            .into_any_element()
    }

    fn menubar(&self) -> AnyElement {
        let recents: Vec<_> = self
            .state
            .workspaces
            .iter()
            .filter(|workspace| !workspace.archived)
            .take(8)
            .map(|workspace| (workspace.id, workspace.display_name()))
            .collect();
        let recent_sender = self.sender.clone();
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("menu-file")
                    .ghost()
                    .small()
                    .label("File")
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu
                            .item(PopupMenuItem::new("New task").disabled(true))
                            .separator()
                            .menu("Open folder…", Box::new(OpenFolder))
                            .menu("Add folder to workspace…", Box::new(AddFolder))
                            .separator();
                        if !recents.is_empty() {
                            menu = menu.item(PopupMenuItem::label("Recent workspaces"));
                            for (id, name) in &recents {
                                let sender = recent_sender.clone();
                                let id = *id;
                                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                    move |_, _, _| {
                                        let _ = sender.send(Command::Select(id));
                                    },
                                ));
                            }
                            menu = menu.separator();
                        }
                        menu.separator()
                            .menu("Preferences…", Box::new(Preferences))
                            .separator()
                            .menu("Exit", Box::new(Exit))
                    }),
            )
            .child(
                Button::new("menu-edit")
                    .ghost()
                    .small()
                    .label("Edit")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Undo").disabled(true))
                            .item(PopupMenuItem::new("Redo").disabled(true))
                            .separator()
                            .item(PopupMenuItem::new("Cut").disabled(true))
                            .item(PopupMenuItem::new("Copy").disabled(true))
                            .item(PopupMenuItem::new("Paste").disabled(true))
                    }),
            )
            .into_any_element()
    }

    fn rename_workspace(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self
            .state
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
        else {
            return;
        };
        let name = workspace.name.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input =
                InputState::new(window, cx).placeholder("Use folder name automatically");
            input.set_value(name, window, cx);
            input
        });
        let sender = self.sender.clone();
        window.focus(&self.focus, cx);
        let focus_input = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input_value = input.clone();
            let sender = sender.clone();
            dialog
                .title("Rename workspace")
                .child(Input::new(&input))
                .child(theme::caption("Leave blank to use the folder name."))
                .footer(dialog_footer("Save name", false))
                .on_ok(move |_, _, cx| {
                    let _ = sender.send(Command::Rename(
                        id,
                        input_value.read(cx).value().to_string(),
                    ));
                    true
                })
        });
        window.on_next_frame(move |window, cx| {
            focus_input.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    fn rename(&mut self, _: &RenameWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.state.active_workspace {
            self.rename_workspace(id, window, cx);
        }
    }

    fn remove_workspace_entry(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .state
            .workspaces
            .iter()
            .any(|workspace| workspace.id == id)
        {
            return;
        }
        let sender = self.sender.clone();
        window.focus(&self.focus, cx);
        window.open_dialog(cx, move |dialog, _, _| {
            let sender = sender.clone();
            dialog
                .title("Remove from recent workspaces?")
                .child("The workspace entry will be removed. Your folders and files stay on disk.")
                .footer(dialog_footer("Remove workspace", true))
                .on_ok(move |_, _, _| {
                    let _ = sender.send(Command::RemoveWorkspace(id));
                    true
                })
        });
    }

    fn remove_workspace(
        &mut self,
        _: &RemoveWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.state.active_workspace {
            self.remove_workspace_entry(id, window, cx);
        }
    }

    fn sidebar(&self, cx: &Context<Self>) -> AnyElement {
        let query = self.search.read(cx).value().to_lowercase();
        let mut list = div()
            .id("workspace-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .py_1()
            .flex()
            .flex_col()
            .gap_1();
        let filtered: Vec<_> = self
            .state
            .workspaces
            .iter()
            .filter(|workspace| !workspace.archived)
            .filter(|w| {
                w.display_name().to_lowercase().contains(&query)
                    || w.roots
                        .iter()
                        .any(|r| r.display_path().to_lowercase().contains(&query))
            })
            .collect();
        if filtered
            .iter()
            .any(|workspace| workspace.pinned && !workspace.archived)
        {
            list = list.child(theme::caption("PINNED"));
            for workspace in filtered
                .iter()
                .filter(|workspace| workspace.pinned && !workspace.archived)
            {
                list = list.child(self.workspace_sidebar_row(workspace, cx));
            }
        }
        if filtered
            .iter()
            .any(|workspace| !workspace.pinned && !workspace.archived)
        {
            list = list.child(theme::caption("RECENT"));
            for workspace in filtered
                .iter()
                .filter(|workspace| !workspace.pinned && !workspace.archived)
            {
                list = list.child(self.workspace_sidebar_row(workspace, cx));
            }
        }
        if filtered.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_3()
                    .child(theme::caption(if query.is_empty() {
                        if self.loaded {
                            "Opened workspaces appear here."
                        } else {
                            "Loading workspaces…"
                        }
                    } else {
                        "No matching workspaces."
                    })),
            );
        }
        div()
            .size_full()
            .bg(rgb(SIDEBAR))
            .flex()
            .flex_col()
            .context_menu(|menu, _, _| {
                menu.menu("Open folder…", Box::new(OpenFolder))
                    .menu("Add folder to workspace…", Box::new(AddFolder))
            })
            .child(
                div().px_3().pt_3().pb_3().child(
                    Input::new(&self.search)
                        .small()
                        .prefix(Icon::new(IconName::Search).small()),
                ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(theme::caption("WORKSPACES"))
                    .child(
                        Button::new("open-sidebar")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .accessibility_label("Open folder")
                            .tooltip("Open folder · Ctrl+O")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_folder(&OpenFolder, window, cx)
                            })),
                    ),
            )
            .child(list)
            .when(
                self.state
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.archived),
                |view| {
                    view.child(
                        div().px_2().pb_1().child(
                            theme::nav_button(
                                "archives",
                                "Archives",
                                Some(Icon::new(IconName::Archive)),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.archives(&Archives, window, cx),
                            )),
                        ),
                    )
                },
            )
            .child(
                div().border_t_1().border_color(rgb(BORDER)).p_2().child(
                    theme::nav_button(
                        "sidebar-account",
                        "Codex Settings",
                        Some(Icon::default().path("icons/codex.svg")),
                    )
                    .accessibility_label("Codex Settings")
                    .tooltip("Codex Settings")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.codex_settings(&OpenCodexSettings, window, cx)
                    })),
                ),
            )
            .into_any_element()
    }

    fn workspace_sidebar_row(
        &self,
        workspace: &crate::workspace::Workspace,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = workspace.id;
        let selected = self.state.active_workspace == Some(id);
        let pinned = workspace.pinned;
        let archived = workspace.archived;
        let pin_sender = self.sender.clone();
        let archive_sender = self.sender.clone();
        let rename_weak = cx.weak_entity();
        let remove_weak = cx.weak_entity();
        div()
            .id(SharedString::from(format!("workspace-row-{id}")))
            .w_full()
            .context_menu(move |menu, _, _| {
                let rename_weak = rename_weak.clone();
                let remove_weak = remove_weak.clone();
                let pin_sender = pin_sender.clone();
                let archive_sender = archive_sender.clone();
                menu.item(
                    PopupMenuItem::new("Rename workspace…").on_click(move |_, window, cx| {
                        let _ = rename_weak
                            .update(cx, |this, cx| this.rename_workspace(id, window, cx));
                    }),
                )
                .item(
                    PopupMenuItem::new(if pinned {
                        "Unpin workspace"
                    } else {
                        "Pin workspace"
                    })
                    .on_click(move |_, _, _| {
                        let _ = pin_sender.send(Command::TogglePin(id));
                    }),
                )
                .item(
                    PopupMenuItem::new(if archived {
                        "Restore from archive"
                    } else {
                        "Archive workspace"
                    })
                    .on_click(move |_, _, _| {
                        let _ = archive_sender.send(Command::ToggleArchive(id));
                    }),
                )
                .separator()
                .item(PopupMenuItem::new("Delete workspace entry…").on_click(
                    move |_, window, cx| {
                        let _ = remove_weak
                            .update(cx, |this, cx| this.remove_workspace_entry(id, window, cx));
                    },
                ))
            })
            .child(
                theme::nav_button(
                    SharedString::from(format!("workspace-{id}")),
                    workspace.display_name(),
                    Some(Icon::new(if workspace.roots.len() > 1 {
                        IconName::Layers
                    } else {
                        IconName::Folder
                    })),
                )
                .h(px(38.))
                .when(selected, |button| {
                    button.bg(rgb(BORDER)).text_color(rgb(TEXT))
                })
                .when(archived, |button| button.text_color(rgb(MUTED)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.send(Command::Select(id));
                    this.search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                })),
            )
            .into_any_element()
    }

    fn folder_row(
        &self,
        workspace: Uuid,
        root: &WorkspaceRoot,
        is_default: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = root.id;
        let availability = self.availability.get(&id).copied();
        let available = availability != Some(false);
        let path = root.display_path();
        let name = root
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let default_sender = self.sender.clone();
        let remove_sender = self.sender.clone();
        let weak = cx.weak_entity();
        div()
            .py_3()
            .flex()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(Icon::new(IconName::Folder).text_color(rgb(if available {
                MUTED
            } else {
                ERROR
            })))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().font_weight(FontWeight::MEDIUM).child(name))
                            .when(is_default, |row| row.child(theme::caption("Default")))
                            .when(availability.is_none(), |row| {
                                row.child(theme::caption("Checking…"))
                            })
                            .when(!available, |row| {
                                row.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(ERROR))
                                        .child("Unavailable"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(path),
                    ),
            )
            .when(!available, |row| {
                row.child(
                    Button::new(SharedString::from(format!("locate-{id}")))
                        .small()
                        .label("Locate…")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_folder(Some((workspace, Some(id))), window, cx)
                        })),
                )
            })
            .child(
                Button::new(SharedString::from(format!("root-menu-{id}")))
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .accessibility_label("Folder actions")
                    .tooltip("Folder actions")
                    .dropdown_menu(move |menu, _, _| {
                        let default_sender = default_sender.clone();
                        let remove_sender = remove_sender.clone();
                        let weak = weak.clone();
                        menu.item(PopupMenuItem::new("Set as default folder").on_click(
                            move |_, _, _| {
                                let _ = default_sender.send(Command::DefaultRoot(workspace, id));
                            },
                        ))
                        .item(PopupMenuItem::new("Locate folder…").on_click(
                            move |_, window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.choose_folder(Some((workspace, Some(id))), window, cx)
                                });
                            },
                        ))
                        .separator()
                        .item(
                            PopupMenuItem::new("Remove folder from workspace").on_click(
                                move |_, _, _| {
                                    let _ = remove_sender.send(Command::RemoveRoot(workspace, id));
                                },
                            ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn task_composer(&self, cx: &Context<Self>) -> AnyElement {
        let status = if self.turn_active {
            "Codex is working"
        } else if self.account.connected {
            "Ready"
        } else {
            "Connecting"
        };
        let access = self.composer_controls.read(cx).access_label().to_owned();
        let mut attachments = div().flex().flex_wrap().gap_2();
        for (index, path) in self.attachments.iter().enumerate() {
            let label = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            attachments = attachments.child(
                Button::new(("attachment", index))
                    .small()
                    .label(label)
                    .icon(IconName::X)
                    .tooltip(format!("Remove {}", path.display()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index < this.attachments.len() {
                            this.attachments.remove(index);
                            cx.notify();
                        }
                    })),
            );
        }
        div()
            .w_full()
            .max_w(px(880.))
            .rounded(px(20.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .when(!self.attachments.is_empty(), |view| view.child(attachments))
            .child(
                div()
                    .min_h(px(88.))
                    .max_h(px(180.))
                    .overflow_y_scrollbar()
                    .child(
                        Textarea::new(&self.composer)
                            .appearance(false)
                            .bordered(false),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("attach-context")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Plus)
                                    .accessibility_label("Attach files or images")
                                    .tooltip("Attach files or images")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.attach_context(window, cx)
                                    })),
                            )
                            .child(theme::caption(access))
                            .child(
                                Button::new("session-status")
                                    .ghost()
                                    .small()
                                    .label(status)
                                    .tooltip("Session and account usage")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.session_status(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.composer_controls.clone())
                            .child(
                                Button::new("send-task")
                                    .primary()
                                    .small()
                                    .icon(if self.turn_active {
                                        IconName::Square
                                    } else {
                                        IconName::ArrowUp
                                    })
                                    .rounded_full()
                                    .size(px(32.))
                                    .accessibility_label(if self.turn_active {
                                        "Stop Codex"
                                    } else {
                                        "Send prompt"
                                    })
                                    .tooltip(if self.turn_active {
                                        "Stop Codex"
                                    } else {
                                        "Send prompt"
                                    })
                                    .disabled(!self.account.connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(workspace) = this.active_turn_workspace {
                                            let _ = this
                                                .account_sender
                                                .send(app_server::Request::Interrupt { workspace });
                                        } else {
                                            this.start_task(window, cx);
                                        }
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn conversation(&self, cx: &Context<Self>) -> AnyElement {
        let current_workspace = self.state.active_workspace;
        let has_messages = self.activity.iter().any(|entry| {
            Some(entry.workspace) == current_workspace
                && (self.state.preferences.show_tool_activity
                    || entry.user
                    || matches!(entry.kind.as_str(), "agentMessage" | "reasoning"))
        });
        if !has_messages {
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().font_weight(FontWeight::MEDIUM).child("Start a task"))
                .child(theme::caption(
                    "Describe the change you want in this workspace. Codex will show its work here as it runs.",
                ))
                .into_any_element();
        }

        let mut messages = div().flex().flex_col().gap_4();
        for entry in self
            .activity
            .iter()
            .filter(|entry| Some(entry.workspace) == current_workspace)
            .filter(|entry| {
                self.state.preferences.show_tool_activity
                    || entry.user
                    || matches!(entry.kind.as_str(), "agentMessage" | "reasoning")
            })
        {
            if entry.user {
                messages = messages.child(
                    div().w_full().flex().justify_end().child(
                        div()
                            .max_w(px(680.))
                            .rounded(px(16.))
                            .bg(rgb(SURFACE))
                            .p_3()
                            .text_color(rgb(TEXT))
                            .child(entry.text.clone()),
                    ),
                );
                continue;
            }

            let is_tool = !matches!(entry.kind.as_str(), "agentMessage" | "reasoning");
            if is_tool {
                let id = entry.id.clone().unwrap_or_default();
                let expanded = self
                    .expanded_activity
                    .contains(&(entry.workspace, id.clone()));
                let summary = entry
                    .text
                    .lines()
                    .next()
                    .unwrap_or("Codex activity")
                    .to_owned();
                let kind = entry.kind.clone();
                let full_text = entry.text.clone();
                let workspace = entry.workspace;
                let mut row =
                    div()
                        .w_full()
                        .max_w(px(820.))
                        .rounded(px(10.))
                        .bg(rgb(BACKGROUND))
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::Ellipsis).text_color(rgb(MUTED)))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(rgb(MUTED))
                                        .child(format!("{} · {}", activity_label(&kind), summary)),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "toggle-activity-{workspace}-{id}"
                                    )))
                                    .ghost()
                                    .small()
                                    .icon(if expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .accessibility_label(if expanded {
                                        "Collapse activity output"
                                    } else {
                                        "Expand activity output"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_activity(workspace, id.clone(), cx)
                                    })),
                                ),
                        );
                if expanded {
                    row = row.child(
                        div()
                            .max_h(px(360.))
                            .overflow_y_scrollbar()
                            .rounded(px(8.))
                            .bg(rgb(BACKGROUND))
                            .p_3()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(full_text),
                    );
                }
                messages = messages.child(row);
            } else {
                messages = messages.child(
                    div()
                        .w_full()
                        .max_w(px(820.))
                        .text_color(rgb(TEXT))
                        .line_height(relative(1.5))
                        .child(TextView::markdown(
                            SharedString::from(format!(
                                "message-{}-{}",
                                entry.workspace,
                                entry.id.as_deref().unwrap_or("local")
                            )),
                            entry.text.clone(),
                        )),
                );
            }
        }
        if let Some(approval) = self
            .pending_approvals
            .iter()
            .find(|approval| Some(approval.workspace) == current_workspace)
        {
            let description = approval.description.clone();
            messages = messages.child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(ACCENT))
                    .bg(rgb(SURFACE))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Codex needs approval"),
                    )
                    .child(theme::caption(description))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("approve-task")
                                    .primary()
                                    .small()
                                    .label("Allow")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.resolve_approval(true, cx)
                                    })),
                            )
                            .child(Button::new("deny-task").small().label("Deny").on_click(
                                cx.listener(|this, _, _, cx| this.resolve_approval(false, cx)),
                            )),
                    ),
            );
        }
        if let Some(view) =
            current_workspace.and_then(|workspace| self.pending_questions.get(&workspace))
        {
            messages = messages.child(view.clone());
        }
        messages.into_any_element()
    }

    fn content(&self, cx: &Context<Self>) -> AnyElement {
        if !self.loaded {
            return div()
                .size_full()
                .p_6()
                .child(theme::caption("Opening your workspace…"))
                .into_any_element();
        }
        let Some(workspace) = self.state.active() else {
            return div().size_full().flex().flex_col()
                .child(div().h(px(60.)).px_6().flex().items_center().border_b_1().border_color(rgb(BORDER)).child(theme::caption("Your development workspace")))
                .child(div().flex_1().flex().items_center().justify_center().px_8()
                    .child(div().max_w(px(430.)).flex().flex_col().gap_3()
                        .child(Icon::new(IconName::FolderOpen).size(px(28.)).text_color(rgb(ACCENT)))
                        .child(div().text_size(px(25.)).font_weight(FontWeight::SEMIBOLD).child("Start with your project."))
                        .child(div().text_color(rgb(MUTED)).line_height(relative(1.6)).child("Open a folder to give your work a home. Bring related repositories together in one workspace."))
                        .child(div().mt_3().flex().gap_3().items_center()
                            .child(Button::new("open-welcome").primary().label("Open folder").icon(IconName::FolderOpen)
                                .on_click(cx.listener(|this, _, window, cx| this.open_folder(&OpenFolder, window, cx))))
                            .child(theme::caption("Ctrl+O")))))
                .child(div().px_6().pb_5().child(theme::caption(if self.account.connected {
                    "Codex is connected. Open a workspace to start a task."
                } else {
                    "Connecting to your local Codex harness…"
                })))
                .into_any_element();
        };
        let mut folders = div().flex().flex_col();
        for root in &workspace.roots {
            folders = folders.child(self.folder_row(
                workspace.id,
                root,
                workspace.default_root == Some(root.id),
                cx,
            ));
        }
        if workspace.roots.is_empty() {
            folders = folders.child(div().py_5().child(theme::caption(
                "No folders in this workspace. Add a folder to continue.",
            )));
        }
        let mut folder_section = div()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .overflow_hidden()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Button::new("toggle-folders")
                            .ghost()
                            .small()
                            .icon(if self.folders_collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .label(format!("Folders  {}", workspace.roots.len()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.folders_collapsed = !this.folders_collapsed;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("add-folder")
                            .ghost()
                            .small()
                            .label("Add folder")
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_folder(&AddFolder, window, cx)
                            })),
                    ),
            );
        if !self.folders_collapsed {
            folder_section = folder_section.child(folders);
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(44.))
                    .px_6()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(workspace.display_name()),
                    )
                    .child(
                        Button::new("workspace-folders-toggle")
                            .ghost()
                            .small()
                            .icon(IconName::Folder)
                            .tooltip("Workspace folders")
                            .accessibility_label("Workspace folders")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.folders_collapsed = !this.folders_collapsed;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("workspace-menu")
                            .ghost()
                            .small()
                            .icon(IconName::Ellipsis)
                            .accessibility_label("Workspace actions")
                            .tooltip("Workspace actions")
                            .dropdown_menu(|menu, _, _| {
                                menu.menu("Rename workspace…", Box::new(RenameWorkspace))
                                    .menu("Add folder…", Box::new(AddFolder))
                                    .menu("Refresh folders", Box::new(RefreshFolders))
                                    .separator()
                                    .menu("Remove from recent…", Box::new(RemoveWorkspace))
                            }),
                    ),
            )
            .child(
                div()
                    .id("workspace-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.conversation_scroll)
                    .p_5()
                    .when(!self.folders_collapsed, |view| view.child(folder_section))
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(880.))
                            .mt_5()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.conversation(cx)),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .pb_3()
                    .flex()
                    .justify_center()
                    .child(self.task_composer(cx)),
            )
            .child(
                div()
                    .h(px(34.))
                    .px_6()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .items_center()
                    .child(theme::caption(if workspace.roots.len() > 1 {
                        "Multi-folder workspace"
                    } else {
                        "Local workspace"
                    })),
            )
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.first_render {
            self.first_render = false;
            crate::diagnostics::mark("first_render");
        }
        let dialogs = Root::render_dialog_layer(window, cx);
        let notifications = Root::render_notification_layer(window, cx);
        let sidebar = self.sidebar(cx);
        let content = self.content(cx);
        div()
            .id("codex-air")
            .key_context("CodexAir")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::switch_workspace))
            .on_action(cx.listener(Self::add_folder))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::remove_workspace))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::clear_search))
            .on_action(cx.listener(Self::preferences))
            .on_action(cx.listener(Self::codex_settings))
            .on_action(cx.listener(Self::about))
            .on_action(cx.listener(Self::check_updates))
            .on_action(cx.listener(Self::release_notes))
            .on_action(cx.listener(Self::archives))
            .on_action(cx.listener(Self::exit))
            .relative()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .font_family("Segoe UI")
            .text_size(px(14.))
            .flex()
            .flex_col()
            .child(self.app_header(cx))
            .when_some(self.warning.clone(), |view, warning| {
                view.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(rgb(SURFACE))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(ERROR))
                                .child(warning),
                        )
                        .child(
                            Button::new("dismiss-warning")
                                .ghost()
                                .small()
                                .icon(IconName::X)
                                .accessibility_label("Dismiss warning")
                                .tooltip("Dismiss")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.warning = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable(if self.loaded {
                        "workspace-panes"
                    } else {
                        "loading-panes"
                    })
                    .child(
                        resizable_panel()
                            .size(px(self.state.sidebar_width))
                            .size_range(px(200.)..px(380.))
                            .child(sidebar),
                    )
                    .child(
                        resizable_panel()
                            .size_range(px(420.)..px(10000.))
                            .child(content),
                    )
                    .on_resize(cx.listener(
                        |this, state: &Entity<gpui_kit::base::ResizableState>, _, cx| {
                            if let Some(width) = state.read(cx).sizes().first() {
                                this.state.sidebar_width = f32::from(*width);
                                this.send(Command::Sidebar(f32::from(*width)));
                            }
                        },
                    )),
                ),
            )
            .children(dialogs)
            .children(notifications)
    }
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenFolder, Some("CodexAir")),
        KeyBinding::new("ctrl-p", SwitchWorkspace, Some("CodexAir")),
        KeyBinding::new("ctrl-shift-o", AddFolder, Some("CodexAir")),
        KeyBinding::new("ctrl-,", Preferences, Some("CodexAir")),
        KeyBinding::new("escape", ClearSearch, Some("CodexAir")),
    ]);
}

fn dialog_footer(label: &'static str, destructive: bool) -> Div {
    div()
        .flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("cancel-dialog")
                .label("Cancel")
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            Button::new("confirm-dialog")
                .primary()
                .when(destructive, |button| button.danger())
                .label(label)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
                }),
        )
}
