//! Embedded static assets (SVG icons) for GPUI.

use std::borrow::Cow;

use anyhow::anyhow;
use gpui::{AssetSource, SharedString};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        Self::get(path)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow!("missing asset: {path}").into())
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| SharedString::from(p.to_string())))
            .collect())
    }
}
