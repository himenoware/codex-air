//! Native settings view backed by the same local App Server as conversations.
use crate::{
    app_server::{AccountStatus, Request},
    theme::*,
};
use gpui_kit::{
    assets::IconName,
    base::Disableable,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        input::{Textarea, TextareaState},
        menu::{DropdownMenu, PopupMenuItem},
    },
    prelude::FluentBuilder,
    *,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc::Sender};

#[derive(Clone, Copy, PartialEq)]
enum Section {
    General,
    Configuration,
    Personalization,
    Usage,
    Mcp,
    Hooks,
    Plugins,
    Account,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Configuration => "Configuration",
            Self::Personalization => "Personalization",
            Self::Usage => "Usage & billing",
            Self::Mcp => "MCP servers",
            Self::Hooks => "Hooks",
            Self::Plugins => "Plugins",
            Self::Account => "Account",
        }
    }
}

pub struct CodexSettings {
    section: Section,
    sender: Sender<Request>,
    cwd: Option<PathBuf>,
    account: AccountStatus,
    config: Value,
    models: Vec<(String, String, Vec<String>)>,
    inventory: Value,
    instructions: Entity<TextareaState>,
    loading: bool,
    message: Option<String>,
    generation: u64,
}

pub struct SettingsChanged;
impl EventEmitter<SettingsChanged> for CodexSettings {}

impl CodexSettings {
    pub fn new(
        sender: Sender<Request>,
        cwd: Option<PathBuf>,
        account: AccountStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let instructions = cx.new(|cx| TextareaState::new(window, cx).rows(6));
        let mut this = Self {
            section: Section::General,
            sender,
            cwd,
            account,
            config: Value::Null,
            models: Vec::new(),
            inventory: Value::Null,
            instructions,
            loading: false,
            message: None,
            generation: 0,
        };
        this.refresh(window, cx);
        this
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.message = None;
        self.generation += 1;
        let generation = self.generation;
        let section = self.section;
        if section == Section::Account {
            let _ = self.sender.send(Request::Refresh);
        }
        let sender = self.sender.clone();
        let cwd = self.cwd.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = match section {
                Section::General | Section::Configuration | Section::Personalization => {
                    inspect(
                        &sender,
                        "config/read",
                        json!({"includeLayers": false, "cwd": cwd}),
                    )
                    .await
                }
                Section::Usage => inspect(&sender, "account/rateLimits/read", Value::Null).await,
                Section::Mcp => {
                    inspect(&sender, "mcpServerStatus/list", json!({"limit":100})).await
                }
                Section::Hooks => {
                    inspect(
                        &sender,
                        "hooks/list",
                        json!({"cwds": cwd.clone().into_iter().collect::<Vec<_>>() }),
                    )
                    .await
                }
                Section::Plugins => {
                    inspect(
                        &sender,
                        "plugin/list",
                        json!({"cwds":cwd.clone().map(|p|vec![p])}),
                    )
                    .await
                }
                Section::Account => {
                    inspect(&sender, "account/read", json!({"refreshToken":false})).await
                }
            };
            let models = if matches!(section, Section::General | Section::Configuration) {
                inspect(&sender, "model/list", json!({"limit":100}))
                    .await
                    .ok()
            } else {
                None
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(value) => {
                        if let Some(config) = value.get("config") {
                            this.config = config.clone();
                            let text = config
                                .get("developer_instructions")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            this.instructions
                                .update(cx, |input, cx| input.set_value(text, window, cx));
                        }
                        if section == Section::Account
                            && let Some(account) = value.get("account")
                        {
                            this.account.connected = !account.is_null();
                            this.account.email = account
                                .get("email")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            this.account.plan = account
                                .get("planType")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                        }
                        this.inventory = value;
                    }
                    Err(error) => this.message = Some(error),
                }
                if let Some(data) = models
                    .and_then(|v| v.get("data").cloned())
                    .and_then(|v| v.as_array().cloned())
                {
                    this.models = data
                        .iter()
                        .filter_map(|model| {
                            let id = model
                                .get("model")
                                .or_else(|| model.get("id"))?
                                .as_str()?
                                .to_owned();
                            let name = model
                                .get("displayName")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_owned();
                            let efforts = model
                                .get("supportedReasoningEfforts")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(|effort| {
                                    effort
                                        .get("reasoningEffort")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned)
                                })
                                .collect();
                            Some((id, name, efforts))
                        })
                        .collect();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn write(
        &mut self,
        key: &'static str,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loading = true;
        self.message = None;
        let sender = self.sender.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = inspect(
                &sender,
                "config/value/write",
                json!({
                    "keyPath":key, "value":value, "mergeStrategy":"replace"
                }),
            )
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(_) => {
                        cx.emit(SettingsChanged);
                        this.refresh(window, cx);
                    }
                    Err(error) => {
                        this.message = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn choice(
        &self,
        key: &'static str,
        label: &'static str,
        detail: &'static str,
        choices: Vec<(String, String)>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let current = self
            .config
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("Default")
            .to_owned();
        let weak = cx.weak_entity();
        row(
            label,
            detail,
            Button::new(key)
                .small()
                .label(current)
                .disabled(self.loading)
                .dropdown_menu(move |mut menu, _, _| {
                    for (value, label) in &choices {
                        let weak = weak.clone();
                        let value = value.clone();
                        menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                            move |_, window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.write(key, json!(value), window, cx)
                                });
                            },
                        ));
                    }
                    menu
                }),
        )
        .into_any_element()
    }

    fn page(&self, cx: &Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_4();
        match self.section {
            Section::General => {
                page = page.child(row(
                    "Agent environment",
                    "Where Codex runs on this computer.",
                    caption("Windows native"),
                ));
                page = page.child(
                    self.choice(
                        "model",
                        "Default model",
                        "Models available to your signed-in account.",
                        self.models
                            .iter()
                            .map(|(id, name, _)| (id.clone(), name.clone()))
                            .collect(),
                        cx,
                    ),
                );
                let model = self.config.get("model").and_then(Value::as_str);
                let efforts = self
                    .models
                    .iter()
                    .find(|(id, _, _)| Some(id.as_str()) == model)
                    .map(|(_, _, efforts)| efforts.clone())
                    .unwrap_or_default();
                if !efforts.is_empty() {
                    page = page.child(self.choice(
                        "model_reasoning_effort",
                        "Reasoning effort",
                        "Used for new Codex tasks.",
                        efforts.into_iter().map(|v| (v.clone(), v)).collect(),
                        cx,
                    ));
                }
                page = page.child(caption(
                    "App appearance and composer shortcuts are in File → Preferences.",
                ));
            }
            Section::Configuration => {
                page = page.child(row(
                    "Custom configuration",
                    "Open your Codex configuration folder.",
                    Button::new("open-config")
                        .small()
                        .label("Show config.toml")
                        .on_click(|_, _, cx| cx.reveal_path(&codex_home().join("config.toml"))),
                ));
                page = page.child(self.choice(
                    "approval_policy",
                    "Approvals",
                    "When Codex asks before running a tool.",
                    choices(&["untrusted", "on-request", "never"]),
                    cx,
                ));
                page = page.child(self.choice(
                    "sandbox_mode",
                    "Sandbox",
                    "Filesystem access for new Codex tasks.",
                    choices(&["read-only", "workspace-write", "danger-full-access"]),
                    cx,
                ));
            }
            Section::Personalization => {
                page=page.child(div().font_weight(FontWeight::MEDIUM).child("Codex instructions"))
                    .child(caption("Additional instructions for your Codex tasks. Repository instructions also apply."))
                    .child(Textarea::new(&self.instructions))
                    .child(Button::new("save-instructions").primary().label("Save instructions").disabled(self.loading).on_click(cx.listener(|this,_,window,cx|{
                        let text=this.instructions.read(cx).value().to_string();
                        this.write("developer_instructions",json!(text),window,cx);
                    })))
                    .child(row("Global instructions","View AGENTS.md in your Codex folder.",Button::new("show-instructions").small().label("Show AGENTS.md").on_click(|_,_,cx|cx.reveal_path(&codex_home().join("AGENTS.md")))));
            }
            Section::Usage => {
                page = page.child(row(
                    "Your plan",
                    "Managed through ChatGPT.",
                    caption(
                        self.account
                            .plan
                            .clone()
                            .unwrap_or_else(|| "Unavailable".into()),
                    ),
                ));
                let limits = self
                    .inventory
                    .get("rateLimitsByLimitId")
                    .and_then(Value::as_object);
                if let Some(limits) = limits {
                    for (id, limit) in limits {
                        page = page.child(limit_view(id, limit));
                    }
                } else if let Some(limit) = self.inventory.get("rateLimits") {
                    page = page.child(limit_view("Codex", limit));
                }
                page = page.child(
                    Button::new("billing")
                        .small()
                        .label("Manage plan and billing")
                        .on_click(|_, _, cx| cx.open_url("https://chatgpt.com/#settings/Account")),
                );
            }
            Section::Mcp => {
                if let Some(servers) = self.inventory.get("data").and_then(Value::as_array) {
                    for server in servers {
                        let name = server
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("MCP server");
                        let status = server
                            .get("authStatus")
                            .and_then(Value::as_str)
                            .unwrap_or("Unknown");
                        let count = server
                            .get("tools")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        page = page.child(row(
                            name,
                            &format!("{count} tools"),
                            caption(status.to_owned()),
                        ));
                    }
                    if servers.is_empty() {
                        page = page.child(caption(
                            "No MCP servers returned by your Codex installation.",
                        ));
                    }
                }
                page = page.child(
                    Button::new("mcp-config")
                        .small()
                        .label("Show configuration")
                        .on_click(|_, _, cx| cx.reveal_path(&codex_home().join("config.toml"))),
                );
            }
            Section::Hooks => {
                let mut count = 0;
                let mut issue_count = 0;
                if let Some(entries) = self.inventory.get("data").and_then(Value::as_array) {
                    for entry in entries {
                        if let Some(hooks) = entry.get("hooks").and_then(Value::as_array) {
                            for hook in hooks {
                                let name = hook
                                    .get("key")
                                    .and_then(Value::as_str)
                                    .or_else(|| hook.get("eventName").and_then(Value::as_str))
                                    .unwrap_or("Hook");
                                let event = hook
                                    .get("eventName")
                                    .and_then(Value::as_str)
                                    .unwrap_or("event");
                                let source = hook
                                    .get("source")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown source");
                                let trust = hook
                                    .get("trustStatus")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown trust");
                                let enabled = hook
                                    .get("enabled")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false);
                                let status = if enabled { "Enabled" } else { "Disabled" };
                                let detail = format!("{event} · {source} · {trust}");
                                page = page.child(row(name, &detail, caption(status)));
                                count += 1;
                            }
                        }
                        if let Some(errors) = entry.get("errors").and_then(Value::as_array) {
                            for error in errors {
                                let path =
                                    error.get("path").and_then(Value::as_str).unwrap_or("hook");
                                let message = error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Hook could not be loaded.");
                                page = page.child(
                                    div()
                                        .text_color(rgb(ERROR))
                                        .child(format!("{path}: {message}")),
                                );
                                issue_count += 1;
                            }
                        }
                        if let Some(warnings) = entry.get("warnings").and_then(Value::as_array) {
                            for warning in warnings.iter().filter_map(Value::as_str) {
                                page = page.child(caption(format!("Hook warning: {warning}")));
                            }
                        }
                    }
                }
                if count == 0 && issue_count == 0 {
                    page = page.child(caption("No hooks are configured for this workspace."));
                }
                page = page.child(
                    Button::new("hook-config")
                        .small()
                        .label("Show configuration")
                        .on_click(|_, _, cx| cx.reveal_path(&codex_home().join("config.toml"))),
                );
            }
            Section::Plugins => {
                let mut count = 0;
                if let Some(markets) = self.inventory.get("marketplaces").and_then(Value::as_array)
                {
                    for market in markets {
                        if let Some(plugins) = market.get("plugins").and_then(Value::as_array) {
                            for plugin in plugins {
                                let name = plugin
                                    .pointer("/interface/displayName")
                                    .or_else(|| plugin.get("name"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("Plugin");
                                let installed = plugin
                                    .get("installed")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false);
                                if installed {
                                    let enabled = plugin
                                        .get("enabled")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false);
                                    let status = if enabled { "Enabled" } else { "Disabled" };
                                    let detail = plugin
                                        .get("disabledReason")
                                        .and_then(Value::as_str)
                                        .map(|reason| format!("Installed plugin · {reason}"))
                                        .unwrap_or_else(|| "Installed plugin".to_owned());
                                    count += 1;
                                    page = page.child(row(name, &detail, caption(status)));
                                }
                            }
                        }
                    }
                }
                if count == 0 {
                    page = page.child(caption(
                        "No installed plugins returned by the local catalog.",
                    ));
                }
                page = page.child(
                    Button::new("plugin-folder")
                        .small()
                        .label("Show plugins folder")
                        .on_click(|_, _, cx| cx.reveal_path(&codex_home().join("plugins"))),
                );
            }
            Section::Account => {
                if let Some(detail) = &self.account.detail {
                    page = page.child(caption(detail.clone()));
                }
                page = page.child(row(
                    "Codex account",
                    "Uses the login managed by your local Codex installation.",
                    caption(
                        self.account
                            .email
                            .clone()
                            .unwrap_or_else(|| "Signed out".into()),
                    ),
                ));
                if !self.account.connected {
                    let sender = self.sender.clone();
                    page = page.child(
                        Button::new("sign-in")
                            .primary()
                            .label("Sign in with ChatGPT")
                            .on_click(move |_, _, _| {
                                let _ = sender.send(Request::StartChatGptLogin);
                            }),
                    );
                }
            }
        }
        page.into_any_element()
    }
}

impl Render for CodexSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div()
            .w(px(180.))
            .flex_shrink_0()
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .bg(rgb(SIDEBAR))
            .rounded_l(px(12.));
        for section in [
            Section::General,
            Section::Configuration,
            Section::Personalization,
            Section::Usage,
            Section::Mcp,
            Section::Hooks,
            Section::Plugins,
            Section::Account,
        ] {
            nav = nav.child(
                nav_button(section.label(), section.label(), None)
                    .when(self.section == section, |button| button.bg(rgb(SURFACE)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.section = section;
                        this.refresh(window, cx);
                    })),
            );
        }
        div().h(px(480.)).w_full().flex().gap_2().child(nav).child(
            div()
                .id("codex-settings-page")
                .flex_1()
                .min_w_0()
                .overflow_y_scroll()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(div().text_size(px(22.)).child(self.section.label()))
                        .child(
                            Button::new("refresh-settings")
                                .ghost()
                                .small()
                                .icon(IconName::RotateCw)
                                .tooltip("Refresh settings")
                                .disabled(self.loading)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                ),
                        ),
                )
                .when(self.loading, |view| view.child(caption("Loading…")))
                .when_some(self.message.clone(), |view, message| {
                    view.child(div().text_color(rgb(ERROR)).child(message))
                })
                .child(self.page(cx)),
        )
    }
}

fn row(label: &str, detail: &str, control: impl IntoElement) -> Div {
    div()
        .py_3()
        .border_b_1()
        .border_color(rgb(BORDER))
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_owned()),
                )
                .child(caption(detail.to_owned())),
        )
        .child(control)
}
fn choices(values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|v| (v.to_string(), v.to_string()))
        .collect()
}
fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".codex")
        })
}
pub fn limit_view(name: &str, value: &Value) -> Div {
    let mut view = div()
        .p_3()
        .rounded(px(12.))
        .bg(rgb(SURFACE))
        .flex()
        .flex_col()
        .gap_2()
        .child(name.to_owned());
    for (key, label) in [
        ("primary", "Current window"),
        ("secondary", "Weekly window"),
    ] {
        if let Some(used) = value
            .pointer(&format!("/{key}/usedPercent"))
            .and_then(Value::as_f64)
        {
            let remaining = (100. - used).clamp(0., 100.);
            let minutes = value
                .pointer(&format!("/{key}/windowDurationMins"))
                .and_then(Value::as_i64);
            let label = match minutes {
                Some(minutes) if minutes >= 1440 => format!("{} day limit", minutes / 1440),
                Some(minutes) if minutes >= 60 => format!("{} hour limit", minutes / 60),
                Some(minutes) => format!("{minutes} minute limit"),
                None => label.to_owned(),
            };
            let reset = value
                .pointer(&format!("/{key}/resetsAt"))
                .and_then(Value::as_u64)
                .and_then(|epoch| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()?
                        .as_secs();
                    let minutes = epoch.saturating_sub(now).div_ceil(60);
                    Some(if minutes >= 1440 {
                        format!("Resets in {}d {}h", minutes / 1440, (minutes % 1440) / 60)
                    } else if minutes >= 60 {
                        format!("Resets in {}h {}m", minutes / 60, minutes % 60)
                    } else {
                        format!("Resets in {minutes}m")
                    })
                })
                .unwrap_or_default();
            view = view
                .child(row(
                    &label,
                    &reset,
                    caption(format!("{remaining:.0}% left")),
                ))
                .child(
                    div().h(px(5.)).rounded_full().bg(rgb(BORDER)).child(
                        div()
                            .h_full()
                            .w(relative((remaining / 100.) as f32))
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    ),
                );
        }
    }
    view
}
async fn inspect(sender: &Sender<Request>, method: &str, params: Value) -> Result<Value, String> {
    let (reply, receiver) = async_channel::bounded(1);
    sender
        .send(Request::Inspect {
            method: method.into(),
            params,
            reply,
        })
        .map_err(|_| "Codex connection closed".to_owned())?;
    receiver
        .recv()
        .await
        .map_err(|_| "Codex connection closed".to_owned())?
}
