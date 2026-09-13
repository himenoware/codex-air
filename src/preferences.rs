use crate::{controller::Command, theme::*, workspace::Preferences};
use gpui_kit::{
    component::{Sizable, button::Button, switch::Switch},
    *,
};
use std::sync::mpsc::Sender;

pub struct PreferencesView {
    value: Preferences,
    sender: Sender<Command>,
}

impl PreferencesView {
    pub fn new(value: Preferences, sender: Sender<Command>) -> Self {
        Self { value, sender }
    }
    fn save(&self) {
        let _ = self.sender.send(Command::Preferences(self.value.clone()));
    }
}
impl Render for PreferencesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(caption("Codex Air preferences are saved on this computer."))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("Enter sends a prompt")
                            .child(caption(
                                "Shift+Enter adds a line. When off, use Ctrl+Enter to send.",
                            )),
                    )
                    .child(
                        Switch::new("enter-sends")
                            .checked(self.value.enter_sends)
                            .on_click(cx.listener(|this, value, _, cx| {
                                this.value.enter_sends = *value;
                                this.save();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("Show tool activity")
                            .child(caption(
                                "Show commands and tool output in the conversation.",
                            )),
                    )
                    .child(
                        Switch::new("tool-activity")
                            .checked(self.value.show_tool_activity)
                            .on_click(cx.listener(|this, value, _, cx| {
                                this.value.show_tool_activity = *value;
                                this.save();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child("Sidebar width")
                    .child(
                        Button::new("reset-sidebar")
                            .small()
                            .label("Reset to default")
                            .on_click(cx.listener(|this, _, _, _| {
                                let _ = this.sender.send(Command::Sidebar(240.));
                                let _ = this.sender.send(Command::Refresh);
                            })),
                    ),
            )
    }
}
