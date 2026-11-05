//! One article as a grid card, with its thumbnail.
//!
//! [`Card`] is a stateless component. Its clicks become [`CardEvent`]s that
//! go to the handler set with [`Card::on_event`]; without one, a live card
//! (`read_only: false`) applies them to the global
//! [`AppModel`](crate::app_model::AppModel) ([`apply_event`]), and a
//! read-only card (Archive) ignores the toggles.
//!
//! Left out by design: a hover lift and a fade-up entrance (GPUI has no
//! element transforms), letter spacing, and italic placeholder initials
//! (only the upright Newsreader is embedded).

use super::icon::IconName;
use super::icon_button;
use crate::app_model::AppModel;
use crate::theme::{self, Tokens};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    canvas, div, fill, hsla, img, point, px, quad, size, App, BorderStyle, Bounds, FontWeight,
    Hsla, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled, StyledImage as _, Window,
};
use radar_core::types::{Article, Theme};
use std::rc::Rc;

/// Display options shared by every card on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardOpts {
    /// The 3 px category bar on the left edge.
    pub show_accent: bool,
    /// Newsreader 19 px headlines instead of Inter 17 px.
    pub use_serif: bool,
    /// Archive display: read/bookmark clicks do nothing.
    pub read_only: bool,
}

/// What a click on a card asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardEvent {
    ToggleRead,
    ToggleBookmark,
    Open,
}

pub type CardHandler = Rc<dyn Fn(CardEvent, &mut Window, &mut App)>;
/// A handler that also gets the article (what the card closures need).
type Emit = Rc<dyn Fn(CardEvent, &mut Window, &mut App, &Article)>;

/// The default effect of a [`CardEvent`] on the global model. `Open` opens
/// the article's URL and changes nothing else (it does not mark the
/// article read).
pub fn apply_event(cx: &mut App, article: &Article, event: CardEvent, read_only: bool) {
    let id = article.id;
    let model = AppModel::global(cx);
    match event {
        CardEvent::ToggleRead | CardEvent::ToggleBookmark if read_only => {}
        CardEvent::ToggleRead => model.update(cx, |m, cx| m.toggle_read(cx, id)),
        CardEvent::ToggleBookmark => model.update(cx, |m, cx| m.toggle_bookmark(cx, id)),
        CardEvent::Open => {
            if article.url.is_empty() {
                return;
            }
            cx.open_url(&article.url);
        }
    }
}

#[derive(IntoElement)]
pub struct Card {
    article: Article,
    opts: CardOpts,
    on_event: Option<CardHandler>,
    scope: SharedString,
}

impl Card {
    pub fn new(article: &Article, opts: CardOpts) -> Self {
        Self {
            article: article.clone(),
            opts,
            on_event: None,
            scope: SharedString::default(),
        }
    }

    /// Namespaces the card's element ids (see [`part_id`]) so the same
    /// article can appear in two places (Saved, an Archive snapshot)
    /// without colliding. Empty (the default) is the live briefing.
    pub fn scope(mut self, scope: impl Into<SharedString>) -> Self {
        self.scope = scope.into();
        self
    }

    #[allow(dead_code)] // tests route events through their own handler
    pub fn on_event(mut self, f: impl Fn(CardEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_event = Some(Rc::new(f));
        self
    }
}

/// Element id of a part of the card of article `id` (`card-7`,
/// `card-7-bookmark`, ...), prefixed `<scope>/` when the card has a scope
/// (`archive-3/card-7`).
pub fn part_id(scope: &str, id: u64, part: &str) -> SharedString {
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}/")
    };
    if part.is_empty() {
        format!("{prefix}card-{id}").into()
    } else {
        format!("{prefix}card-{id}-{part}").into()
    }
}

/// Up to two initials of the source name (`Dezeen` -> `D`, `It's Nice
/// That` -> `IN`): the first character of each space-separated word.
fn initials(source: &str) -> String {
    source
        .split(' ')
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect()
}

impl RenderOnce for Card {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = *theme::t(cx);
        let theme_mode = AppModel::global(cx).read(cx).model.prefs.theme;
        let Card {
            article: a,
            opts,
            on_event,
            scope,
        } = self;
        let ns: &str = &scope;
        let emit = move |ev: CardEvent, w: &mut Window, cx: &mut App, a: &Article| match &on_event {
            Some(h) => h(ev, w, cx),
            None => apply_event(cx, a, ev, opts.read_only),
        };
        let emit = Rc::new(emit);
        let accent = t.dot_for(&a.accent);

        let hover_shadow = theme::shadow_md(theme_mode);
        let (border, border_strong) = (t.border, t.border_strong);
        let mut card = div()
            .id(part_id(ns, a.id, ""))
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .min_h(px(180.))
            .overflow_hidden()
            .bg(t.surface)
            .border_1()
            .border_color(border)
            .rounded(t.radius)
            .cursor_pointer()
            .opacity(if a.read { 0.7 } else { 1. })
            .hover(move |s| s.border_color(border_strong).shadow(hover_shadow.clone()))
            .on_click({
                let (emit, a) = (emit.clone(), a.clone());
                move |_, w, cx| emit(CardEvent::ToggleRead, w, cx, &a)
            });
        if opts.show_accent {
            if let Some(c) = accent {
                card = card.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(3.))
                        .rounded_tl(t.radius)
                        .rounded_bl(t.radius)
                        .bg(c),
                );
            }
        }
        card.child(thumb(&a, ns, &t))
            .child(body(&a, ns, opts, &t, theme_mode, emit))
    }
}

/// The card body and everything in it.
fn body(
    a: &Article,
    ns: &str,
    opts: CardOpts,
    t: &Tokens,
    _theme: Theme,
    emit: Emit,
) -> impl IntoElement {
    let accent = t.dot_for(&a.accent);
    let meta = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .mb(px(10.))
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .font_family(theme::FONT_MONO)
        .text_size(px(11.5))
        .text_color(t.text_3)
        .children(accent.map(|c| dot(c, 8.)))
        .child(
            div()
                .id(part_id(ns, a.id, "source"))
                .min_w_0()
                .truncate()
                .text_color(t.text_2)
                .font_weight(FontWeight::MEDIUM)
                .child(a.source.clone()),
        )
        .child(div().flex_none().child("·"))
        .child(
            div()
                .min_w_0()
                .flex_shrink(1.)
                .truncate()
                .text_size(px(10.5))
                .child(a.category_label.to_uppercase()),
        )
        .child(
            div()
                .flex_none()
                .ml_auto()
                .pl(px(6.))
                .child(a.published.clone()),
        );

    // `font-size: 19/17` with `line-height: 1.35`. Long titles stop at four
    // lines so a degenerate headline can't stretch the card.
    let size = if opts.use_serif { 19. } else { 17. };
    let mut headline = div()
        .id(part_id(ns, a.id, "title"))
        .test_support()
        .min_w_0()
        .mb(px(10.))
        .text_size(px(size))
        .line_height(gpui_kit::relative(1.35))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text)
        .line_clamp(4)
        .child(a.title.clone());
    if opts.use_serif {
        headline = headline.font_family(theme::FONT_SERIF);
    }

    // An empty summary leaves no element (and no 14 px margin) behind.
    let summary = (!a.summary.trim().is_empty()).then(|| {
        div()
            .id(part_id(ns, a.id, "summary"))
            .test_support()
            .min_w_0()
            .mb(px(14.))
            .text_size(px(13.5))
            .line_height(gpui_kit::relative(1.55))
            .text_color(t.text_2)
            .line_clamp(3)
            .child(a.summary.clone())
    });

    let bookmark = {
        let label = if a.bookmarked {
            "Remove bookmark"
        } else {
            "Bookmark"
        };
        let (emit, a) = (emit.clone(), a.clone());
        icon_button(
            part_id(ns, a.id, "bookmark"),
            if a.bookmarked {
                IconName::BookmarkFilled
            } else {
                IconName::Bookmark
            },
            13.,
            false,
            t,
        )
        .size(px(26.))
        .test_support()
        .aria_label(label)
        .on_click(move |_, w, cx| {
            cx.stop_propagation();
            emit(CardEvent::ToggleBookmark, w, cx, &a)
        })
    };
    let open = {
        let (emit, a) = (emit, a.clone());
        let has_url = !a.url.is_empty();
        let b = icon_button(part_id(ns, a.id, "open"), IconName::External, 13., false, t)
            .size(px(26.))
            .test_support()
            .aria_label("Open in browser");
        // `disabled={!article.url}`
        if has_url {
            b.on_click(move |_, w, cx| {
                cx.stop_propagation();
                emit(CardEvent::Open, w, cx, &a)
            })
            .into_any_element()
        } else {
            b.cursor_default().into_any_element()
        }
    };

    let footer = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .mt_auto()
        .pt(px(10.))
        .border_t_1()
        .border_dashed()
        .border_color(t.border)
        .text_size(px(12.))
        .text_color(t.text_3)
        .children((!a.read).then(|| {
            div()
                .id(part_id(ns, a.id, "unread-dot"))
                .test_support()
                .flex_none()
                .size(px(7.))
                .rounded_full()
                .bg(t.accent)
        }))
        .child(if a.read { "Read" } else { "New" })
        .child(
            div()
                .ml_auto()
                .flex()
                .gap(px(2.))
                .child(bookmark)
                .child(open),
        );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .p(t.density_pad)
        .child(meta)
        .child(headline)
        .children(summary)
        .child(footer)
}

fn dot(color: Hsla, d: f32) -> gpui_kit::Div {
    div().flex_none().size(px(d)).rounded_full().bg(color)
}

/// The thumbnail: the cover image, or a decorative placeholder.
fn thumb(a: &Article, ns: &str, t: &Tokens) -> impl IntoElement {
    let label = div()
        .absolute()
        .bottom(px(8.))
        .left(px(10.))
        .px(px(6.))
        .py(px(2.))
        .rounded(px(3.))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .font_family(theme::FONT_MONO)
        .text_size(px(9.5))
        .text_color(t.text_3)
        .child(format!("cover · {}", a.category_label).to_uppercase());
    let frame = div()
        .id(part_id(ns, a.id, "thumb"))
        .test_support()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(96.))
        .overflow_hidden()
        .border_b_1()
        .border_color(t.border);
    let placeholder_of = {
        let (a, t, ns) = (a.clone(), *t, ns.to_string());
        move || placeholder(&a, &ns, &t)
    };
    match a.image_url.as_deref().filter(|u| !u.is_empty()) {
        Some(url) => {
            let fallback = placeholder_of.clone();
            frame
                .child(
                    img(url.to_string())
                        .id(part_id(ns, a.id, "image"))
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        // A failed load -> the same placeholder as no image.
                        .with_fallback(move || fallback().into_any_element()),
                )
                .child(label)
                .into_any_element()
        }
        None => frame
            .child(placeholder_of().into_any_element())
            .child(label)
            .into_any_element(),
    }
}

/// The decorative thumb for feeds without an image: a pattern picked by
/// `id % 5` in the category color, with the source's initials on top.
/// Fills its parent.
fn placeholder(a: &Article, ns: &str, t: &Tokens) -> impl IntoElement {
    let color = t.dot_for(&a.accent).unwrap_or(t.text_3);
    let kind = Pattern::of(a.id);
    div()
        .id(part_id(ns, a.id, "placeholder"))
        .test_support()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.surface_2)
        .opacity(0.9)
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| paint_pattern(kind, bounds, color, window),
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .px(px(14.))
                .py(px(2.))
                .border_1()
                .border_color(t.border)
                .rounded(px(4.))
                .bg(t.surface)
                .font_family(theme::FONT_SERIF)
                .text_size(px(32.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(hsla(color.h, color.s, color.l, color.a * 0.55))
                .child(initials(&a.source)),
        )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pattern {
    Stripes,
    Grid,
    Dots,
    Diag,
    Arcs,
}

impl Pattern {
    fn of(id: u64) -> Self {
        [
            Self::Stripes,
            Self::Grid,
            Self::Dots,
            Self::Diag,
            Self::Arcs,
        ][(id % 5) as usize]
    }
}

/// The five background patterns of the placeholder, painted as quads and
/// strokes.
fn paint_pattern(
    kind: Pattern,
    bounds: Bounds<gpui_kit::Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let rect = |x: f32, y: f32, rw: f32, rh: f32| Bounds::new(at(x, y), size(px(rw), px(rh)));
    match kind {
        // `repeating-linear-gradient(90deg, c 0 1px, transparent 1px 14px)`
        Pattern::Stripes => {
            let mut x = 0.;
            while x < w {
                window.paint_quad(fill(rect(x, 0., 1., h), color));
                x += 14.;
            }
        }
        // Horizontal and vertical lines every 18 px.
        Pattern::Grid => {
            let mut y = 0.;
            while y < h {
                window.paint_quad(fill(rect(0., y, w, 1.), color));
                y += 18.;
            }
            let mut x = 0.;
            while x < w {
                window.paint_quad(fill(rect(x, 0., 1., h), color));
                x += 18.;
            }
        }
        // `radial-gradient(c 1px, transparent 1.4px) 0 0 / 14px 14px`: a dot
        // in the middle of every 14 px tile.
        Pattern::Dots => {
            let mut y = 0.;
            while y < h {
                let mut x = 0.;
                while x < w {
                    window.paint_quad(quad(
                        rect(x + 6., y + 6., 2., 2.),
                        px(1.),
                        color,
                        0.,
                        gpui_kit::transparent_black(),
                        BorderStyle::default(),
                    ));
                    x += 14.;
                }
                y += 14.;
            }
        }
        // `repeating-linear-gradient(45deg, c 0 1px, transparent 1px 12px)`:
        // lines running top-left to bottom-right, 12 px apart.
        Pattern::Diag => {
            let step = 12. * std::f32::consts::SQRT_2;
            let mut x0 = -h;
            while x0 < w {
                let mut path = gpui_kit::PathBuilder::stroke(px(1.));
                path.move_to(at(x0, 0.));
                path.line_to(at(x0 + h, h));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
                x0 += step;
            }
        }
        // Two non-repeating radial gradients: a 1 px dot at each bottom
        // corner.
        Pattern::Arcs => {
            for x in [0., w] {
                window.paint_quad(quad(
                    rect(x - 1., h - 1., 2., 2.),
                    px(1.),
                    color,
                    0.,
                    gpui_kit::transparent_black(),
                    BorderStyle::default(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::testing::{host, Host};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{px, TestAppContext};

    fn article(id: u64) -> Article {
        Article {
            id,
            title: "A headline".into(),
            source: "Dezeen".into(),
            source_id: 1,
            category: "tools".into(),
            category_label: "Tools".into(),
            summary: "Some summary".into(),
            published: "1h ago".into(),
            published_at: None,
            read: false,
            bookmarked: false,
            accent: "olive".into(),
            url: "https://x.test/1".into(),
            image_url: None,
            summarized_by: None,
        }
    }

    const OPTS: CardOpts = CardOpts {
        show_accent: true,
        use_serif: true,
        read_only: false,
    };

    fn show(cx: &mut TestAppContext, a: Article) -> Host {
        host(cx, 360., move |_, _| Card::new(&a, OPTS).into_any_element())
    }

    #[test]
    fn initials_from_the_name() {
        assert_eq!(initials("Dezeen"), "D");
        assert_eq!(initials("It's Nice That"), "IN");
        assert_eq!(initials("A  B"), "AB");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn pattern_is_picked_by_id() {
        assert_eq!(Pattern::of(0), Pattern::Stripes);
        assert_eq!(Pattern::of(7), Pattern::Dots);
        assert_eq!(Pattern::of(9), Pattern::Arcs);
    }

    #[gpui_kit::test]
    fn renders_degraded_article(cx: &mut TestAppContext) {
        let a = Article {
            title: "W".repeat(600),
            summary: String::new(),
            image_url: None,
            ..article(1)
        };
        let h = show(cx, a);
        h.with_window(cx, |window| {
            // No image: the decorative placeholder.
            assert!(window.try_find("card-1-placeholder").is_some());
            assert!(window.try_find("card-1-image").is_none());
            // The thumb keeps its 96 px height.
            assert_eq!(window.find("card-1-thumb").bounds().size.height, px(96.));
            // An empty summary leaves no element, hence no blank gap.
            assert!(window.try_find("card-1-summary").is_none());

            // A 600-char title stays inside the card and stops at 4 lines.
            let card = window.find("card-1").bounds();
            let title = window.find("card-1-title").bounds();
            assert_eq!(card.size.width, px(360.), "the card keeps its width");
            assert!(title.right() <= card.right(), "{title:?} vs {card:?}");
            assert!(title.left() >= card.left());
            assert!(
                title.size.height <= px(19. * 1.35 * 4. + 1.),
                "title not clamped: {:?}",
                title.size
            );
        });
    }

    #[gpui_kit::test]
    fn whitespace_summary_is_treated_as_empty(cx: &mut TestAppContext) {
        let h = show(
            cx,
            Article {
                summary: "  \n ".into(),
                ..article(1)
            },
        );
        h.with_window(cx, |window| {
            assert!(window.try_find("card-1-summary").is_none());
        });
    }

    #[gpui_kit::test]
    fn summary_shows_when_present(cx: &mut TestAppContext) {
        let h = show(cx, article(1));
        h.with_window(cx, |window| {
            assert!(window.try_find("card-1-summary").is_some());
        });
    }

    /// The test app's HTTP client fails every request, so the image never
    /// loads and the `with_fallback` placeholder takes its place.
    #[gpui_kit::test]
    fn card_broken_image_uses_fallback(cx: &mut TestAppContext) {
        let h = show(
            cx,
            Article {
                image_url: Some("http://127.0.0.1:9/cover.png".into()),
                ..article(1)
            },
        );
        cx.executor().allow_parking();
        for _ in 0..200 {
            cx.run_until_parked();
            let found = h.with_window(cx, |window| window.try_find("card-1-placeholder").is_some());
            if found {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the fallback never replaced the failed image");
    }

    #[gpui_kit::test]
    fn read_only_card_ignores_toggles_but_opens_urls(cx: &mut TestAppContext) {
        let a = article(1);
        let h = host(cx, 360., move |_, _| {
            Card::new(
                &a,
                CardOpts {
                    read_only: true,
                    ..OPTS
                },
            )
            .into_any_element()
        });
        h.add_live_article(cx, article(1));
        h.click(cx, "card-1");
        h.click(cx, "card-1-bookmark");
        h.live_article(cx, |a| assert!(!a.read && !a.bookmarked));
        h.click(cx, "card-1-open");
        assert_eq!(cx.opened_url().as_deref(), Some("https://x.test/1"));
        h.live_article(cx, |a| assert!(!a.read, "opening never changes read state"));
    }

    #[gpui_kit::test]
    fn custom_handler_receives_events(cx: &mut TestAppContext) {
        use std::cell::RefCell;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let a = article(1);
        let h = host(cx, 360., move |_, _| {
            let log = log.clone();
            Card::new(&a, OPTS)
                .on_event(move |ev, _, _| log.borrow_mut().push(ev))
                .into_any_element()
        });
        h.click(cx, "card-1");
        h.click(cx, "card-1-bookmark");
        h.click(cx, "card-1-open");
        assert_eq!(
            *seen.borrow(),
            [
                CardEvent::ToggleRead,
                CardEvent::ToggleBookmark,
                CardEvent::Open
            ]
        );
    }

    #[gpui_kit::test]
    fn card_without_url_has_inert_open_button(cx: &mut TestAppContext) {
        let h = show(
            cx,
            Article {
                url: String::new(),
                ..article(1)
            },
        );
        h.click(cx, "card-1-open");
        assert_eq!(cx.opened_url(), None);
    }
}
