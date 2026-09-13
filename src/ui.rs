use crate::{
    app_server::{self, AccountStatus},
    controller::{self, Command},
    platform,
    theme::{self, *},
    workspace::{AppState, WorkspaceRoot},
};
use gpui_kit::{
    assets::IconName,
    base::{Disableable, h_resizable, resizable_panel},
    component::{
        Icon, Root, Sizable, TitleBar, WindowExt,
        button::{Button, ButtonVariants},
        dialog::Confirm,
        input::{Input, InputEvent, InputState},
        menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
    },
    prelude::FluentBuilder,
    *,
};
use std::{collections::HashMap, sync::mpsc::Sender};
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
        Exit
    ]
);

pub struct Shell {
    state: AppState,
    availability: HashMap<Uuid, bool>,
    sender: Sender<Command>,
    search: Entity<InputState>,
    composer: Entity<InputState>,
    focus: FocusHandle,
    loaded: bool,
    warning: Option<String>,
    closing: bool,
    first_render: bool,
    can_save: bool,
    availability_generation: u64,
    account: AccountStatus,
    account_sender: Sender<app_server::Request>,
    thread_id: Option<String>,
    turn_active: bool,
    activity: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, updates) = controller::start();
        let (account_sender, account_updates) = app_server::start();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a workspace…"));
        let composer = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Ask Codex to work on this workspace…")
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
                if matches!(event, InputEvent::PressEnter { .. }) {
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
                        {
                            platform::restore(window, saved);
                        }
                        this.state = snapshot.state;
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
                    .update_in(cx, |this, _, cx| {
                        match update {
                            app_server::Update::Status(status) => this.account = status,
                            app_server::Update::LoginUrl(url) => cx.open_url(&url),
                            app_server::Update::ThreadStarted {
                                workspace,
                                thread_id,
                            } => {
                                this.thread_id = Some(thread_id.clone());
                                this.send(Command::SetThread(workspace, thread_id));
                            }
                            app_server::Update::Activity(activity) => {
                                this.activity.push(activity);
                                if this.activity.len() > 80 {
                                    self::Shell::trim_activity(&mut this.activity);
                                }
                            }
                            app_server::Update::TurnFinished => this.turn_active = false,
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
            focus,
            loaded: false,
            warning: None,
            closing: false,
            first_render: true,
            can_save: true,
            availability_generation: 0,
            account: AccountStatus::default(),
            account_sender,
            thread_id: None,
            turn_active: false,
            activity: Vec::new(),
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

    fn trim_activity(activity: &mut Vec<String>) {
        let excess = activity.len().saturating_sub(80);
        if excess > 0 {
            activity.drain(0..excess);
        }
    }

    fn start_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.turn_active {
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
        let text = self.composer.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.turn_active = true;
        self.activity.push(format!("You: {text}"));
        let _ = self.account_sender.send(app_server::Request::StartTurn {
            workspace: workspace_id,
            cwd,
            text,
            thread_id,
        });
        cx.notify();
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
        let account = self.account.clone();
        let sender = self.account_sender.clone();
        window.focus(&self.focus, cx);
        window.open_dialog(cx, move |dialog, _, _| {
            let sender = sender.clone();
            let connected = account.connected;
            let identity = account.email.clone().unwrap_or_else(|| {
                if connected {
                    "Connected through the local Codex App Server.".into()
                } else {
                    "No ChatGPT account is connected to the local Codex App Server.".into()
                }
            });
            dialog
                .title("Preferences")
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().font_weight(FontWeight::MEDIUM).child("Codex account"))
                        .child(
                            div()
                                .text_color(rgb(if connected { ACCENT } else { MUTED }))
                                .child(identity),
                        )
                        .when_some(account.plan.clone(), |view, plan| {
                            view.child(theme::caption(format!("ChatGPT {plan}")))
                        })
                        .when_some(account.detail.clone(), |view, detail| {
                            view.child(theme::caption(detail))
                        }),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-preferences")
                                .label("Close")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("account-preferences-action")
                                .primary()
                                .label(if connected {
                                    "Refresh"
                                } else {
                                    "Sign in with ChatGPT"
                                })
                                .on_click(move |_, _, _| {
                                    let _ = sender.send(if connected {
                                        app_server::Request::Refresh
                                    } else {
                                        app_server::Request::StartChatGptLogin
                                    });
                                }),
                        ),
                )
        });
    }

    fn exit(&mut self, _: &Exit, window: &mut Window, _: &mut Context<Self>) {
        // Keep the action local to the current native window. The regular
        // close path remains responsible for placement/state persistence.
        window.remove_window();
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
                    .gap_2()
                    .flex_1()
                    .child(
                        div()
                            .text_color(rgb(ACCENT))
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(16.))
                            .child("///"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .child("Codex Air"),
                    ),
            )
            .child(div().h(px(18.)).border_l_1().border_color(rgb(BORDER)))
            .child(self.menubar())
            .child(div().h(px(18.)).border_l_1().border_color(rgb(BORDER)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_color(rgb(TEXT))
                    .child(workspace),
            )
            .into_any_element()
    }

    fn menubar(&self) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("menu-file")
                    .ghost()
                    .small()
                    .label("File")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("New task").disabled(true))
                            .separator()
                            .menu("Open folder…", Box::new(OpenFolder))
                            .menu("Add folder to workspace…", Box::new(AddFolder))
                            .separator()
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
            .child(
                Button::new("menu-selection")
                    .ghost()
                    .small()
                    .label("Selection")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Select all").disabled(true))
                            .item(PopupMenuItem::new("Expand selection").disabled(true))
                    }),
            )
            .child(
                Button::new("menu-view")
                    .ghost()
                    .small()
                    .label("View")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Appearance").disabled(true))
                            .item(PopupMenuItem::new("Command palette").disabled(true))
                    }),
            )
            .child(
                Button::new("menu-go")
                    .ghost()
                    .small()
                    .label("Go")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Back").disabled(true))
                            .item(PopupMenuItem::new("Forward").disabled(true))
                    }),
            )
            .child(
                Button::new("menu-run")
                    .ghost()
                    .small()
                    .label("Run")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Start task").disabled(true))
                            .item(PopupMenuItem::new("Stop task").disabled(true))
                    }),
            )
            .child(
                Button::new("menu-terminal")
                    .ghost()
                    .small()
                    .label("Terminal")
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("New terminal").disabled(true))
                            .item(PopupMenuItem::new("Split terminal").disabled(true))
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
        if filtered.iter().any(|workspace| workspace.archived) {
            list = list.child(theme::caption("ARCHIVED"));
            for workspace in filtered.iter().filter(|workspace| workspace.archived) {
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
            .child(
                div().border_t_1().border_color(rgb(BORDER)).p_2().child(
                    Button::new("sidebar-account")
                        .ghost()
                        .w_full()
                        .justify_start()
                        .icon(IconName::User)
                        .label(self.account.email.clone().unwrap_or_else(|| {
                            if self.account.connected {
                                "Codex connected".into()
                            } else {
                                "Connect Codex".into()
                            }
                        }))
                        .accessibility_label("Codex account and preferences")
                        .tooltip(if self.account.connected {
                            "Codex connected · Preferences"
                        } else {
                            "Connect Codex · Preferences"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.preferences(&Preferences, window, cx)
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
                Button::new(SharedString::from(format!("workspace-{id}")))
                    .ghost()
                    .w_full()
                    .h(px(38.))
                    .justify_start()
                    .icon(if workspace.roots.len() > 1 {
                        IconName::Layers
                    } else {
                        IconName::Folder
                    })
                    .label(workspace.display_name())
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
        div()
            .mt_8()
            .p_3()
            .rounded_md()
            .bg(rgb(BORDER))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::MEDIUM).child("New task"))
                    .child(theme::caption(status)),
            )
            .child(Input::new(&self.composer).small())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(theme::caption("Work locally · Enter to send"))
                    .child(
                        Button::new("send-task")
                            .primary()
                            .small()
                            .label(if self.turn_active {
                                "Working…"
                            } else {
                                "Send"
                            })
                            .disabled(self.turn_active || !self.account.connected)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_task(window, cx)),
                            ),
                    ),
            )
            .when(!self.activity.is_empty(), |view| {
                let mut activity = div()
                    .mt_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(MUTED))
                    .flex()
                    .flex_col()
                    .gap_1();
                for entry in self.activity.iter().rev().take(8).rev() {
                    activity = activity.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .line_clamp(2)
                            .child(entry.clone()),
                    );
                }
                view.child(activity)
            })
            .into_any_element()
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
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(60.))
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
                    .p_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pb_2()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .items_center()
                                    .child(div().font_weight(FontWeight::MEDIUM).child("Folders"))
                                    .child(theme::caption(workspace.roots.len().to_string())),
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
                    )
                    .child(folders)
                    .child(
                        div()
                            .mt_10()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Codex"))
                            .child(
                                div()
                                    .text_color(rgb(if self.account.connected {
                                        ACCENT
                                    } else {
                                        MUTED
                                    }))
                                    .child(if self.account.connected {
                                        "Connected through your local Codex harness."
                                    } else {
                                        "Connecting to your local Codex harness…"
                                    }),
                            )
                            .child(theme::caption(if self.account.connected {
                                "This workspace is ready for a Codex task."
                            } else {
                                "Account status will appear when the local App Server responds."
                            })),
                    )
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
            .on_action(cx.listener(Self::exit))
            .relative()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .font_family("Segoe UI")
            .text_size(px(14.))
            .flex()
            .flex_col()
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
            .child(self.app_header(cx))
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
