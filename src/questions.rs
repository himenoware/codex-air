use crate::{
    app_server::{Question, Request},
    theme::*,
};
use gpui_kit::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    },
    *,
};
use std::sync::mpsc::Sender;

pub struct QuestionView {
    id: String,
    questions: Vec<Question>,
    inputs: Vec<Entity<InputState>>,
    sender: Sender<Request>,
    answered: bool,
    error: Option<String>,
}

impl QuestionView {
    pub fn new(
        id: String,
        questions: Vec<Question>,
        sender: Sender<Request>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = questions
            .iter()
            .map(|_| cx.new(|cx| InputState::new(window, cx).placeholder("Your answer…")))
            .collect();
        Self {
            id,
            questions,
            inputs,
            sender,
            answered: false,
            error: None,
        }
    }
    fn answer(&mut self, skip: bool, cx: &mut Context<Self>) {
        let answers = self
            .questions
            .iter()
            .zip(&self.inputs)
            .map(|(question, input)| {
                let text = input.read(cx).value().trim().to_owned();
                (
                    question.id.clone(),
                    if skip || text.is_empty() {
                        Vec::new()
                    } else {
                        vec![text]
                    },
                )
            })
            .collect();
        match self.sender.send(Request::AnswerQuestions {
            id: self.id.clone(),
            answers,
        }) {
            Ok(()) => self.answered = true,
            Err(_) => {
                self.error = Some("Codex disconnected before your answer could be sent.".into())
            }
        }
        cx.notify();
    }
}

impl Render for QuestionView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.answered {
            return div()
                .child(caption("Response sent to Codex"))
                .into_any_element();
        }
        let mut body = div()
            .w_full()
            .p_4()
            .rounded(px(16.))
            .bg(rgb(SURFACE))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Codex has a question"),
            );
        for (index, question) in self.questions.iter().enumerate() {
            let mut choices = div().flex().flex_wrap().gap_2();
            for (option_index, option) in question.options.iter().enumerate() {
                let text = option.clone();
                choices = choices.child(
                    Button::new(SharedString::from(format!(
                        "answer-option-{index}-{option_index}"
                    )))
                    .small()
                    .label(option.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.inputs[index]
                            .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                    })),
                );
            }
            body = body
                .child(question.question.clone())
                .child(choices)
                .child(Input::new(&self.inputs[index]));
        }
        if let Some(error) = &self.error {
            body = body.child(caption(error.clone()));
        }
        body.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("skip-questions")
                        .ghost()
                        .label("Skip")
                        .on_click(cx.listener(|this, _, _, cx| this.answer(true, cx))),
                )
                .child(
                    Button::new("submit-answers")
                        .primary()
                        .label("Continue")
                        .on_click(cx.listener(|this, _, _, cx| this.answer(false, cx))),
                ),
        )
        .into_any_element()
    }
}
