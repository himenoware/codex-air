use gpui_kit::{
    component::{Theme, ThemeMode, theme::ThemeTokens},
    *,
};

pub const BACKGROUND: u32 = 0x1e1e2e;
pub const SIDEBAR: u32 = 0x181825;
pub const SURFACE: u32 = 0x242436;
pub const BORDER: u32 = 0x313244;
pub const TEXT: u32 = 0xcdd6f4;
pub const MUTED: u32 = 0xa6adc8;
pub const ACCENT: u32 = 0x89b4fa;
pub const ERROR: u32 = 0xf38ba8;

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = "Segoe UI".into();
    theme.font_size = px(14.);
    theme.radius = px(5.);
    theme.radius_lg = px(8.);
    theme.background = rgb(BACKGROUND).into();
    theme.foreground = rgb(TEXT).into();
    theme.border = rgb(BORDER).into();
    theme.muted = rgb(SURFACE).into();
    theme.muted_foreground = rgb(MUTED).into();
    theme.accent = rgb(BORDER).into();
    theme.accent_foreground = rgb(TEXT).into();
    theme.primary = rgb(ACCENT).into();
    theme.primary_foreground = rgb(SIDEBAR).into();
    theme.primary_hover = rgb(0xb4d0fc).into();
    theme.primary_active = rgb(0x74a4ed).into();
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
