//! Shell components (ports of `src/components/*.jsx`).
//!
//! Each shell part is a function the [`Workspace`](crate::workspace::Workspace)
//! calls from its render with the current [`AppModel`](crate::app_model::AppModel).
//! Model changes go through `AppModel::mutate`; workspace-local state
//! (search, Tweaks, sources intent) through `cx.listener`.

pub mod card;
pub mod header;
pub mod icon;
pub mod row;
pub mod sidebar;
pub mod skeleton;
pub mod toggle;
pub mod tweaks;

use crate::theme::Tokens;
use gpui_kit::{
    div, px, Div, ElementId, InteractiveElement as _, ParentElement as _, Stateful, Styled,
};
use icon::{Icon, IconName};

/// The `.iconbtn` class of `styles.css`: 32×32, `--radius`, text-2;
/// hover: surface-2 and text; `.active`: accent-text on accent-soft (and
/// no hover change, as `.active` wins the CSS tie).
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    icon_size: f32,
    active: bool,
    t: &Tokens,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(t.radius)
        .cursor_pointer()
        .child(Icon::new(icon).size(px(icon_size)));
    if active {
        el.text_color(t.accent_text).bg(t.accent_soft)
    } else {
        let (hover_bg, hover_text) = (t.surface_2, t.text);
        el.text_color(t.text_2)
            .hover(move |s| s.bg(hover_bg).text_color(hover_text))
    }
}

#[cfg(test)]
pub mod testing {
    //! A one-element host window for component tests.
    use crate::app_model::AppModel;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        div, px, AnyElement, AnyWindowHandle, AppContext as _, Context, Entity, IntoElement,
        ParentElement as _, Render, Styled, TestAppContext, Window, WindowOptions,
    };
    use radar_core::types::Article;

    type Build = Box<dyn Fn(&mut Window, &mut Context<Root>) -> AnyElement>;

    pub struct Root {
        width: f32,
        build: Build,
    }

    impl Render for Root {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .w(px(self.width))
                .child((self.build)(window, cx))
        }
    }

    pub struct Host {
        _dir: tempfile::TempDir,
        window: AnyWindowHandle,
        model: Entity<AppModel>,
    }

    /// Opens a window showing `build()` in a column `width` px wide, with
    /// an [`AppModel`] (theme tokens) behind it.
    pub fn host(
        cx: &mut TestAppContext,
        width: f32,
        build: impl Fn(&mut Window, &mut Context<Root>) -> AnyElement + 'static,
    ) -> Host {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let (window, model) = cx.update(|cx| {
            gpui_kit::init(cx);
            let model = AppModel::init(path, None, cx);
            let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Root {
                    width,
                    build: Box::new(build),
                })
            })
            .unwrap();
            (window, model)
        });
        cx.run_until_parked();
        Host {
            _dir: dir,
            window,
            model,
        }
    }

    impl Host {
        pub fn with_window<R>(
            &self,
            cx: &mut TestAppContext,
            f: impl FnOnce(&mut Window) -> R,
        ) -> R {
            cx.update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                f(window)
            })
            .unwrap()
        }

        pub fn click(&self, cx: &mut TestAppContext, id: &'static str) {
            cx.update_window(self.window, |_, window, cx| window.click(id, cx))
                .unwrap();
            cx.run_until_parked();
        }

        pub fn add_live_article(&self, cx: &mut TestAppContext, a: Article) {
            self.model
                .update(cx, |m, cx| m.update_model(cx, |r| r.articles.push(a)));
        }

        pub fn live_article(&self, cx: &mut TestAppContext, f: impl FnOnce(&Article)) {
            self.model.read_with(cx, |m, _| f(&m.model.articles[0]));
        }
    }
}
