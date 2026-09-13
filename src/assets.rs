use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/codex.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/codex.svg"))));
        }
        if path == "icons/air.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/air.svg"))));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        if "icons/codex.svg".starts_with(path) {
            paths.push("icons/codex.svg".into());
        }
        if "icons/air.svg".starts_with(path) {
            paths.push("icons/air.svg".into());
        }
        Ok(paths)
    }
}
