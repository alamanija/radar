//! The icon set of `src/components/Icon.jsx`, drawn from embedded SVGs in
//! `assets/icons/ui/` (paths copied verbatim from the JSX).
//!
//! `Icon::new(IconName::Search).size(px(14.))` paints in the inherited text
//! color (CSS `currentColor`); set `text_color` on a parent to change it.
//!
//! To add an icon: drop `<name>.svg` into `assets/icons/ui/` (24×24 viewBox,
//! `stroke="currentColor"`), then add a variant to [`IconName`] and one row
//! to the `icons!` table below. [`AppAssets`] serves it at
//! `icons/ui/<name>.svg`; the `every_icon_is_embedded` test covers the table.

use gpui_kit::{
    percentage, px, svg, Animation, AnimationExt as _, App, AssetSource, IntoElement, Pixels,
    RenderOnce, SharedString, Styled, Transformation, Window,
};
use std::borrow::Cow;
use std::time::Duration;

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IconName { $($variant),* }

        impl IconName {
            pub const ALL: &'static [IconName] = &[$(IconName::$variant),*];

            /// Asset path, as served by [`AppAssets`].
            pub fn path(self) -> &'static str {
                match self { $(IconName::$variant => concat!("icons/ui/", $file, ".svg")),* }
            }

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(IconName::$variant => include_bytes!(concat!("../../assets/icons/ui/", $file, ".svg"))),*
                }
            }
        }
    };
}

icons! {
    Radar => "radar",
    Search => "search",
    Sparkles => "sparkles",
    Refresh => "refresh",
    Grid => "grid",
    List => "list",
    Sun => "sun",
    Moon => "moon",
    Settings => "settings",
    Bookmark => "bookmark",
    BookmarkFilled => "bookmark-filled",
    Inbox => "inbox",
    Archive => "archive",
    Feed => "feed",
    Plus => "plus",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronDown => "chevron-down",
    DotV => "dot-v",
    Check => "check",
    External => "external",
    Circle => "circle",
    CircleFilled => "circle-filled",
    Eye => "eye",
    Sliders => "sliders",
    X => "x",
    Tag => "tag",
    Trash => "trash",
}

/// The app's asset source: the UI icons above, falling back to the kit's
/// bundled Lucide set (used by gpui-kit components).
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = IconName::ALL.iter().find(|i| i.path() == path) {
            return Ok(Some(Cow::Borrowed(icon.bytes())));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = IconName::ALL
            .iter()
            .map(|i| i.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect();
        out.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(out)
    }
}

/// One icon. Default size 16 px (the JSX default).
#[derive(gpui_kit::IntoElement)]
pub struct Icon {
    name: IconName,
    size: Pixels,
    spin: bool,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: px(16.),
            spin: false,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// The `.spin` class: one turn per second, forever.
    pub fn spin(mut self, spin: bool) -> Self {
        self.spin = spin;
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        // GPUI's svg paints only with its own text color, so resolve
        // `currentColor` from the inherited text style here.
        let color = window.text_style().color;
        let el = svg()
            .path(self.name.path())
            .size(self.size)
            .flex_none()
            .text_color(color);
        if self.spin {
            el.with_animation(
                "spin",
                Animation::new(Duration::from_secs(1)).repeat(),
                |el, delta| el.with_transformation(Transformation::rotate(percentage(delta))),
            )
            .into_any_element()
        } else {
            el.into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_embedded() {
        for icon in IconName::ALL {
            let bytes = AppAssets.load(icon.path()).unwrap().expect("embedded");
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.starts_with("<svg "), "{icon:?}");
            assert!(text.contains(r#"viewBox="0 0 24 24""#), "{icon:?}");
        }
        assert_eq!(IconName::ALL.len(), 28);
    }

    #[test]
    fn falls_back_to_kit_icons() {
        let lucide = gpui_kit::assets::IconName::Search;
        assert!(AppAssets.load(&lucide.path()).unwrap().is_some());
        // Like the kit's own source, a missing path is an error, not data.
        assert!(!matches!(AppAssets.load("icons/ui/nope.svg"), Ok(Some(_))));
    }
}
