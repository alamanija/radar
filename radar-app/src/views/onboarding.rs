//! The first-run
//! walkthrough shown inline on the briefing view. Progress is inferred from
//! the model (name set, categories, sources), so the user sees it fill in.

use crate::app_model::AppModel;
use crate::components::icon::{Icon, IconName};
use crate::theme::{self, Tokens};
use crate::workspace::{SourcesIntent, Workspace};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    div, px, relative, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled,
};
use radar_core::types::View;

#[derive(Clone, Copy)]
enum Cta {
    Go(View),
    AddSource,
}

struct Step {
    n: u32,
    title: &'static str,
    body: &'static str,
    done: bool,
    cta: &'static str,
    on_click: Cta,
}

fn steps(app: &AppModel) -> [Step; 4] {
    let m = &app.model;
    [
        Step {
            n: 1,
            title: "Tell Radar who you are",
            body: "Your name, role, and a short \"lens\" help Claude weigh what's relevant to you.",
            done: !m.profile.name.trim().is_empty(),
            cta: "Open Profile",
            on_click: Cta::Go(View::Settings),
        },
        Step {
            n: 2,
            title: "Create categories",
            body: "Categories are buckets Claude sorts stories into — e.g. Packaging, Tools, Industry. Describe each one clearly.",
            done: !m.categories.is_empty(),
            cta: "Add categories",
            on_click: Cta::Go(View::Categories),
        },
        Step {
            n: 3,
            title: "Add RSS sources",
            body: "Paste a few feed URLs for the sites you already follow. Radar fetches them in parallel each briefing.",
            done: !m.sources.is_empty(),
            cta: "Add sources",
            on_click: Cta::AddSource,
        },
        Step {
            n: 4,
            title: "Run your briefing",
            body: "Press Morning Briefing in the top bar. Optional: add your Anthropic API key in Settings → Integrations for AI summaries.",
            done: false,
            cta: "Open Settings",
            on_click: Cta::Go(View::Settings),
        },
    ]
}

pub fn render(app: &AppModel, cx: &Context<Workspace>) -> impl IntoElement {
    let t: Tokens = *theme::t(cx);
    let steps = steps(app);
    let remaining = steps.iter().filter(|s| !s.done).count();

    let lede = if remaining == 0 {
        "You're set up — press Morning Briefing at the top to fetch your first briefing."
    } else {
        "A short walkthrough to get your first briefing on the page."
    };
    let header = div()
        .flex()
        .items_start()
        .gap(px(14.))
        .pb(px(18.))
        .mb(px(6.))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(36.))
                .rounded(px(8.))
                .bg(t.accent_soft)
                .text_color(t.accent_text)
                .child(Icon::new(IconName::Radar).size(px(18.))),
        )
        .child(
            div()
                .child(
                    div()
                        .my(px(2.))
                        .mb(px(4.))
                        .font_family(theme::FONT_SERIF)
                        .text_size(px(22.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Welcome to Radar."),
                )
                .child(
                    div()
                        .text_size(px(13.5))
                        .line_height(relative(1.5))
                        .text_color(t.text_2)
                        .child(lede),
                ),
        );

    let list = steps.into_iter().map(|s| {
        let n = s.n;
        let num = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(26.))
            .rounded_full()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .font_features(theme::font_features(true))
            .bg(if s.done { t.accent_soft } else { t.surface_2 })
            .text_color(if s.done { t.accent_text } else { t.text_2 })
            .child(if s.done {
                Icon::new(IconName::Check).size(px(14.)).into_any_element()
            } else {
                n.to_string().into_any_element()
            });
        let cta = (!s.done).then(|| {
            let on_click = s.on_click;
            div()
                .id(format!("onboarding-cta-{n}"))
                .test_support()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .h(px(30.))
                .px(px(12.))
                .rounded(px(6.))
                .border_1()
                .border_color(t.border_strong)
                .bg(t.surface_2)
                .text_size(px(12.5))
                .text_color(t.text)
                .cursor_pointer()
                .on_click(cx.listener(move |ws, _, _, cx| match on_click {
                    Cta::Go(view) => AppModel::mutate(cx, |m| m.prefs.view = view),
                    Cta::AddSource => {
                        ws.sources_intent = Some(SourcesIntent::Add);
                        cx.notify();
                        AppModel::mutate(cx, |m| m.prefs.view = View::Sources);
                    }
                }))
                .child(s.cta)
                .child(Icon::new(IconName::ChevronRight).size(px(13.)))
        });
        div()
            .id(format!("onboarding-step-{n}"))
            .test_support()
            .aria_label(if s.done { "done" } else { "todo" })
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .border_b_1()
            .border_color(t.border)
            .opacity(if s.done { 0.55 } else { 1. })
            .child(num)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .mb(px(2.))
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(s.title),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(relative(1.5))
                            .text_color(t.text_2)
                            .child(s.body),
                    ),
            )
            .children(cta)
    });

    div()
        .id("onboarding")
        .test_support()
        .pt(px(28.))
        .px(px(28.))
        .pb(px(18.))
        .border_1()
        .border_color(t.border)
        .rounded(t.radius)
        .bg(t.surface)
        .child(header)
        .child(div().children(list))
}
