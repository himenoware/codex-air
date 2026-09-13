use gpui_kit::{
    component::{
        Icon, Theme, ThemeMode,
        button::{Button, ButtonVariants},
        theme::ThemeTokens,
    },
    prelude::FluentBuilder,
    *,
};

pub const BACKGROUND: u32 = 0x202127;
pub const SIDEBAR: u32 = 0x191a20;
pub const SURFACE: u32 = 0x2b2d35;
pub const BORDER: u32 = 0x383a44;
pub const TEXT: u32 = 0xe5e5eb;
pub const MUTED: u32 = 0xa5a7b5;
pub const ACCENT: u32 = 0x89b4fa;
pub const ERROR: u32 = 0xf38ba8;

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = "Segoe UI".into();
    theme.font_size = px(14.);
    theme.radius = px(8.);
    theme.radius_lg = px(16.);
    theme.background = rgb(BACKGROUND).into();
    theme.foreground = rgb(TEXT).into();
    theme.border = rgb(BORDER).into();
    theme.muted = rgb(SURFACE).into();
    theme.muted_foreground = rgb(MUTED).into();
    theme.accent = rgb(BORDER).into();
    theme.accent_foreground = rgb(TEXT).into();
    theme.primary = rgb(TEXT).into();
    theme.primary_foreground = rgb(SIDEBAR).into();
    theme.primary_hover = rgb(0xffffff).into();
    theme.primary_active = rgb(0xcdced8).into();
    theme.secondary = rgb(SURFACE).into();
    theme.secondary_foreground = rgb(TEXT).into();
    theme.input = rgb(SIDEBAR).into();
    theme.ring = rgb(ACCENT).into();
    theme.selection = rgba(0x89b4fa40).into();
    theme.popover = rgb(SURFACE).into();
    theme.popover_foreground = rgb(TEXT).into();
    theme.danger = rgb(ERROR).into();
    theme.button = rgb(SURFACE).into();
    theme.button_foreground = rgb(TEXT).into();
    theme.button_hover = rgb(BORDER).into();
    theme.button_active = rgb(0x45475a).into();
    theme.button_primary = theme.primary;
    theme.button_primary_foreground = theme.primary_foreground;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.tokens = ThemeTokens::from(&theme.colors);
    Theme::sync_base(cx);
}

pub fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text.into())
}

pub fn nav_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<Icon>,
) -> Button {
    let label = label.into();
    Button::new(id)
        .ghost()
        .w_full()
        .h(px(36.))
        .accessibility_label(label.clone())
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .when_some(icon, |row, icon| row.child(icon.size(px(16.))))
                .child(div().flex_1().min_w_0().truncate().child(label)),
        )
}
