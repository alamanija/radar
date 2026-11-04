//! Visual tokens (port of `src/styles.css` + `ACCENTS` in `src/App.jsx`) and
//! their projection onto the gpui-kit `Theme`.
//!
//! Views read colors and metrics from [`t`]; gpui-kit components read the
//! mapped subset through `cx.theme()`. Both are refreshed together by
//! [`apply`], which `AppModel::update_model` calls whenever the theme,
//! accent or density pref changes.

use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::{px, App, Global, Hsla, Pixels};
use radar_core::types::{Accent, Density, Prefs, Theme};

/// UI family (name inside `assets/fonts/Inter-Variable.ttf`).
pub const FONT_SANS: &str = "Inter";
/// Headline serif (`.serif` in `styles.css`).
pub const FONT_SERIF: &str = "Newsreader";
/// Mono family (`.mono` in `styles.css`).
pub const FONT_MONO: &str = "JetBrains Mono";

/// Every color and metric token of `styles.css` that views need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_text: Hsla,
    /// Category dots, in order: olive, rust, slate, plum, sand, ink.
    pub dot: [Hsla; 6],
    pub row_h: Pixels,
    pub density_pad: Pixels,
    pub radius: Pixels,
    pub radius_lg: Pixels,
}

impl Global for Tokens {}

fn hex(v: u32) -> Hsla {
    gpui_kit::rgb(v).into()
}

/// `ACCENTS[name]` from `App.jsx`: `(l, d, sl, sd, tl, td)`.
fn accent_hex(accent: Accent) -> [u32; 6] {
    match accent {
        Accent::Camel => [0x9A7B5B, 0xC4A882, 0xEEE5D8, 0x2A2419, 0x6B5338, 0xD9BE93],
        Accent::Olive => [0x6F7A3A, 0xA2A468, 0xEAECDB, 0x22241A, 0x4B5327, 0xC3C586],
        Accent::Rust => [0xA1573A, 0xCB8862, 0xF1DED1, 0x2A1E17, 0x6E3B26, 0xE0A184],
        Accent::Slate => [0x556170, 0x8896A6, 0xE1E4E8, 0x1D2127, 0x3A4450, 0xA8B4C2],
        Accent::Ink => [0x222428, 0xC8CBD1, 0xE3E4E6, 0x1D1F23, 0x1A1A1A, 0xE8E6E3],
    }
}

/// Tokens for one theme/accent/density combination. Values are verbatim
/// from `src/styles.css` (`:root`, `[data-theme="dark"]`,
/// `[data-density="compact"]`) and `ACCENTS` in `src/App.jsx`.
pub fn tokens(theme: Theme, accent: Accent, density: Density) -> Tokens {
    let [l, d, sl, sd, tl, td] = accent_hex(accent);
    let (accent, accent_soft, accent_text) = match theme {
        Theme::Light => (l, sl, tl),
        Theme::Dark => (d, sd, td),
    };
    let (row_h, density_pad) = match density {
        Density::Cozy => (px(44.), px(20.)),
        Density::Compact => (px(36.), px(14.)),
    };
    let base = match theme {
        Theme::Light => [
            0xF5F3F0, 0xFFFFFF, 0xFAF8F5, 0xE8E4DD, 0xD9D3C8, 0x1A1A1A, 0x6B6B6B, 0x9A9590,
        ],
        Theme::Dark => [
            0x0F1114, 0x161920, 0x1A1E26, 0x262A32, 0x333842, 0xE8E6E3, 0x9AA0A9, 0x6A6F78,
        ],
    };
    let dot = match theme {
        Theme::Light => [0x8C8E52, 0xB06A43, 0x6C7A89, 0x8C6079, 0xC9A96B, 0x3E4147],
        Theme::Dark => [0xA2A468, 0xCB8862, 0x8896A6, 0xA77A93, 0xD8B880, 0xB8BCC4],
    };
    Tokens {
        bg: hex(base[0]),
        surface: hex(base[1]),
        surface_2: hex(base[2]),
        border: hex(base[3]),
        border_strong: hex(base[4]),
        text: hex(base[5]),
        text_2: hex(base[6]),
        text_3: hex(base[7]),
        accent: hex(accent),
        accent_soft: hex(accent_soft),
        accent_text: hex(accent_text),
        dot: dot.map(hex),
        row_h,
        density_pad,
        radius: px(6.),
        radius_lg: px(10.),
    }
}

/// The current tokens. Panics if [`apply`] has not run yet (it runs in
/// `AppModel::init`, before any window opens).
pub fn t(cx: &App) -> &Tokens {
    cx.global::<Tokens>()
}

/// Stores the tokens for `prefs` as a global and maps them onto the
/// gpui-kit theme (when gpui-kit is initialized; headless model tests skip
/// `gpui_kit::init`). Idempotent; refreshes every window.
pub fn apply(cx: &mut App, prefs: &Prefs) {
    let tk = tokens(prefs.theme, prefs.accent, prefs.density);
    cx.set_global(tk);
    if !cx.has_global::<KitTheme>() {
        return;
    }
    let mode = match prefs.theme {
        Theme::Light => ThemeMode::Light,
        Theme::Dark => ThemeMode::Dark,
    };
    // Switching mode loads the registered kit theme over the colors, so it
    // must happen in its own step before our colors go on top.
    if KitTheme::global(cx).mode != mode {
        KitTheme::change(mode, None, cx);
    }
    KitTheme::update(cx, |th| {
        th.background = tk.bg;
        th.foreground = tk.text;
        th.border = tk.border;
        th.primary = tk.accent;
        th.primary_foreground = tk.surface;
        th.ring = tk.accent;
        th.muted_foreground = tk.text_2;
        th.popover = tk.surface;
        th.popover_foreground = tk.text;
        th.font_family = FONT_SANS.into();
        th.mono_font_family = FONT_MONO.into();
        th.radius = tk.radius;
        th.radius_lg = tk.radius_lg;
    });
}

/// Embeds the bundled OFL fonts. Call once, before the first window.
///
/// Upstream (google/fonts) only ships variable TTFs for these families, so
/// these are the `wght` (and `opsz`) variable files; each registers under
/// its plain family name.
pub fn register_fonts(cx: &mut App) {
    let fonts = vec![
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Variable.ttf").as_slice()),
        std::borrow::Cow::Borrowed(
            include_bytes!("../assets/fonts/Newsreader-Variable.ttf").as_slice(),
        ),
        std::borrow::Cow::Borrowed(
            include_bytes!("../assets/fonts/JetBrainsMono-Variable.ttf").as_slice(),
        ),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        log::error!("[theme] failed to register bundled fonts: {e:#}");
        return;
    }
    let names = cx.text_system().all_font_names();
    for family in [FONT_SANS, FONT_SERIF, FONT_MONO] {
        if names.iter().any(|n| n == family) {
            log::info!("[theme] font family {family:?} available");
        } else {
            log::warn!("[theme] font family {family:?} missing after registration");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::rgb;

    fn h(v: u32) -> Hsla {
        rgb(v).into()
    }

    #[test]
    fn light_camel_tokens() {
        let t = tokens(Theme::Light, Accent::Camel, Density::Cozy);
        assert_eq!(t.bg, h(0xF5F3F0));
        assert_eq!(t.accent, h(0x9A7B5B));
        assert_eq!(t.accent_soft, h(0xEEE5D8));
        assert_eq!(t.accent_text, h(0x6B5338));
        assert_eq!(t.dot[0], h(0x8C8E52));
        assert_eq!(t.dot[5], h(0x3E4147));
        assert_eq!(t.row_h, px(44.));
        assert_eq!(t.density_pad, px(20.));
        assert_eq!(t.radius, px(6.));
        assert_eq!(t.radius_lg, px(10.));
    }

    #[test]
    fn dark_slate_tokens() {
        let t = tokens(Theme::Dark, Accent::Slate, Density::Cozy);
        assert_eq!(t.accent, h(0x8896A6));
        assert_eq!(t.accent_text, h(0xA8B4C2));
        assert_eq!(t.accent_soft, h(0x1D2127));
        assert_eq!(t.bg, h(0x0F1114));
        assert_eq!(t.text, h(0xE8E6E3));
        assert_eq!(t.dot[3], h(0xA77A93));
    }

    #[test]
    fn compact_density() {
        let t = tokens(Theme::Light, Accent::Camel, Density::Compact);
        assert_eq!(t.row_h, px(36.));
        assert_eq!(t.density_pad, px(14.));
    }
}
