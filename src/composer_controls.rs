//! Per-task controls populated from the installed Codex harness.
use crate::app_server::Request;
use gpui_kit::{
    assets::IconName,
    base::Disableable,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        menu::{DropdownMenu, PopupMenuItem},
    },
    *,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc::Sender};

struct Model {
    id: String,
    name: String,
    efforts: Vec<String>,
    default_effort: Option<String>,
}

pub struct ComposerControls {
    sender: Sender<Request>,
    cwd: Option<PathBuf>,
    models: Vec<Model>,
    model: Option<String>,
    effort: Option<String>,
    access: String,
    loading: bool,
    error: Option<String>,
    generation: u64,
}

impl ComposerControls {
    pub fn new(sender: Sender<Request>, cwd: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            sender,
            cwd,
            models: Vec::new(),
            model: None,
            effort: None,
            access: "Configured access".into(),
            loading: false,
            error: None,
            generation: 0,
        };
        this.refresh(cx);
        this
    }

    pub fn set_workspace(&mut self, cwd: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.cwd != cwd {
            self.cwd = cwd;
            self.model = None;
            self.effort = None;
            self.refresh(cx);
        }
    }

    pub fn selection(&self) -> (Option<String>, Option<String>) {
        (self.model.clone(), self.effort.clone())
    }

    pub fn access_label(&self) -> &str {
        &self.access
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        let sender = self.sender.clone();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let config = inspect(
                &sender,
                "config/read",
                json!({"includeLayers":false,"cwd":cwd}),
            )
            .await;
            let catalog = inspect(&sender, "model/list", json!({"limit":100})).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match config {
                    Ok(value) => {
                        let config = &value["config"];
                        this.model = config["model"].as_str().map(str::to_owned);
                        this.effort = config["model_reasoning_effort"].as_str().map(str::to_owned);
                        this.access = match config["sandbox_mode"].as_str() {
                            Some("danger-full-access") => "Full access",
                            Some("workspace-write") => "Workspace access",
                            Some("read-only") => "Read only",
                            _ => "Configured access",
                        }
                        .into();
                    }
                    Err(error) => this.error = Some(error),
                }
                match catalog {
                    Ok(value) => {
                        this.models = value["data"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|value| {
                                let id = value["model"].as_str()?.to_owned();
                                Some(Model {
                                    name: value["displayName"].as_str().unwrap_or(&id).to_owned(),
                                    id,
                                    efforts: value["supportedReasoningEfforts"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter_map(|v| {
                                            v["reasoningEffort"].as_str().map(str::to_owned)
                                        })
                                        .collect(),
                                    default_effort: value["defaultReasoningEffort"]
                                        .as_str()
                                        .map(str::to_owned),
                                })
                            })
                            .collect();
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ComposerControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .models
            .iter()
            .find(|model| Some(&model.id) == self.model.as_ref());
        let model_name = selected
            .map(|m| m.name.clone())
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| {
                if self.loading {
                    "Loading models…"
                } else {
                    "Default model"
                }
                .into()
            });
        let models: Vec<_> = self
            .models
            .iter()
            .map(|m| (m.id.clone(), m.name.clone(), m.default_effort.clone()))
            .collect();
        let efforts = selected.map(|m| m.efforts.clone()).unwrap_or_default();
        let model_weak = cx.weak_entity();
        let effort_weak = cx.weak_entity();
        let tooltip = self
            .error
            .clone()
            .unwrap_or_else(|| "Model for this conversation".into());
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("composer-model")
                    .ghost()
                    .small()
                    .label(model_name)
                    .icon(IconName::ChevronDown)
                    .tooltip(tooltip)
                    .disabled(self.loading)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, name, effort) in &models {
                            let id = id.clone();
                            let effort = effort.clone();
                            let weak = model_weak.clone();
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.model = Some(id.clone());
                                        this.effort = effort.clone();
                                        cx.notify();
                                    });
                                },
                            ));
                        }
                        let weak = model_weak.clone();
                        menu.separator()
                            .item(
                                PopupMenuItem::new("Refresh models").on_click(move |_, _, cx| {
                                    let _ = weak.update(cx, |this, cx| this.refresh(cx));
                                }),
                            )
                    }),
            )
            .child(
                Button::new("composer-effort")
                    .ghost()
                    .small()
                    .label(
                        self.effort
                            .clone()
                            .unwrap_or_else(|| "Default effort".into()),
                    )
                    .icon(IconName::ChevronDown)
                    .disabled(self.loading || efforts.is_empty())
                    .tooltip("Reasoning effort")
                    .dropdown_menu(move |mut menu, _, _| {
                        for effort in &efforts {
                            let value = effort.clone();
                            let weak = effort_weak.clone();
                            menu = menu.item(PopupMenuItem::new(effort.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.effort = Some(value.clone());
                                        cx.notify();
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
    }
}

pub async fn inspect(
    sender: &Sender<Request>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let (reply, response) = async_channel::bounded(1);
    sender
        .send(Request::Inspect {
            method: method.to_owned(),
            params,
            reply,
        })
        .map_err(|_| "Codex is not connected.".to_owned())?;
    response
        .recv()
        .await
        .map_err(|_| "Codex disconnected.".to_owned())?
}

pub struct StatusView {
    thread_id: Option<String>,
    usage: Option<(i64, Option<i64>)>,
    limits: Option<Value>,
    error: Option<String>,
}

impl StatusView {
    pub fn new(
        sender: Sender<Request>,
        thread_id: Option<String>,
        usage: Option<(i64, Option<i64>)>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            let result = inspect(&sender, "account/rateLimits/read", Value::Null).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(value) => this.limits = Some(value),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            thread_id,
            usage,
            limits: None,
            error: None,
        }
    }
}

impl Render for StatusView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        use crate::theme::*;
        let context = match self.usage {
            Some((used, Some(total))) if total > 0 => format!(
                "{:.0}% context left · {used} / {total} tokens",
                (100.0 - used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
            ),
            Some((used, _)) => format!("{used} tokens used · context capacity unavailable"),
            None => "Context usage appears after Codex reports a turn.".into(),
        };
        let mut content = div()
            .id("session-status-content")
            .max_h(px(420.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .child(caption(
                self.thread_id
                    .clone()
                    .map(|id| format!("Session: {id}"))
                    .unwrap_or_else(|| "New session".into()),
            ))
            .child(context);
        if let Some(limits) = &self.limits {
            if let Some(by_id) = limits["rateLimitsByLimitId"].as_object() {
                for (id, limit) in by_id {
                    content = content.child(crate::codex_settings::limit_view(id, limit));
                }
            } else if !limits["rateLimits"].is_null() {
                content = content.child(crate::codex_settings::limit_view(
                    "Codex",
                    &limits["rateLimits"],
                ));
            } else {
                content = content.child(caption("Usage limits are unavailable for this account."));
            }
        } else if let Some(error) = &self.error {
            content = content.child(caption(error.clone()));
        } else {
            content = content.child(caption("Loading account usage…"));
        }
        content
    }
}
