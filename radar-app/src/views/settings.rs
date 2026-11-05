//! Port of `src/views/SettingsView.jsx`: Account, Profile, Briefing,
//! Curation, Appearance, Updates and Integrations, in that order, with the
//! JSX copy.
//!
//! # State
//! [`SettingsView`] lives on the `Workspace` and is dropped whenever another
//! view is shown, as the JSX controls' state died on unmount. On show,
//! [`SettingsView::prepare`] builds the inputs, reads the API key status
//! (keychain, off the UI thread) and the login-item state, and keeps the
//! controlled inputs (name, role, schedule time) in step with the model.
//!
//! # Secrets
//! The API key control only ever holds `{present, preview}`. The typed key
//! sits in its masked input until Save, then goes straight to the keychain
//! on the blocking pool, and the input is dropped once that succeeds.
//!
//! # Differences from the JSX
//! - Clerk's `<SignInButton>`/`<UserButton>` become the browser sign-in
//!   pill and an avatar with a "Sign out" pill.
//! - `<input type="time">` is the kit `TimeField`; the lens `<textarea>` is
//!   the kit `Textarea`.

use super::briefing::{page_frame, view_subtitle};
use super::form::{block, new_input, text_input, Pill, PillStyle};
use crate::app_model::AppModel;
use crate::bridge;
use crate::components::icon::IconName;
use crate::components::toggle::Toggle;
use crate::platform::{autostart, updater};
use crate::theme::{self, Tokens};
use crate::workspace::Workspace;
use chrono::NaiveTime;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::time_field::{TimeField, TimeFieldEvent, TimeFieldState};
use gpui_kit::component::{Disableable as _, Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, img, px, AnyElement, App, AppContext as _, Context, Div, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled, StyledImage as _, Subscription, Window,
};
use radar_core::keychain;
use radar_core::types::{Account, Density, Theme};

const SUBTITLE: &str = "Radar runs locally by default. Sign in to sync your sources, \
                        categories, profile, and read state across devices.";
const LENS_PLACEHOLDER: &str = "e.g. Brand studio founder focused on packaging and editorial \
                                illustration. Prefer craft over trend.";
/// `settingsStyles.wrap` max width.
const MAX_WIDTH: f32 = 880.;
/// 13 px Inter at `line-height: normal`.
const LENS_LINE_HEIGHT: f32 = 15.75;
/// The lens editor's `width: 100%; maxWidth: 420` as the browser resolves
/// it on the 880 px page: the 420 px basis shrinks with the label column
/// (389 px of sub text) to fit the 778 px row. Taffy measures an auto-basis
/// text column differently, so the resolved width is set directly.
const LENS_EDITOR_WIDTH: f32 = 404.;

#[derive(Default)]
pub struct SettingsView {
    /// Built on first show.
    pub form: Option<Form>,
}

pub struct Form {
    pub name: Entity<InputState>,
    pub role: Entity<InputState>,
    pub schedule: Entity<TimeFieldState>,
    /// The lens textarea while editing (`LensControl`).
    pub lens: Option<Entity<TextareaState>>,
    pub api: ApiKey,
    /// Login-item state as the OS reports it (`AutostartToggle`).
    pub autostart: bool,
    /// The model values the controlled fields last agreed on.
    synced: Synced,
    _subscriptions: Vec<Subscription>,
}

/// The last value each controlled field and the model agreed on. A model
/// value that differs from it was changed elsewhere (a sync pull, Tweaks)
/// and is shown; a field edit updates it before the model, so a frame
/// drawn between a keystroke and its change event never undoes the typing.
#[derive(Clone, PartialEq)]
struct Synced {
    name: String,
    role: String,
    time: String,
}

#[derive(Clone, Copy)]
enum Field {
    Name,
    Role,
}

/// `ApiKeyControl`'s state. Only the status, never the key.
#[derive(Default)]
pub struct ApiKey {
    pub present: bool,
    pub preview: Option<String>,
    /// The masked input while adding or replacing a key.
    pub draft: Option<Entity<InputState>>,
    pub busy: bool,
    pub error: Option<String>,
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    let (h, m) = radar_core::schedule::parse_time(s)?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Shows `value` (`set_value` emits no change, so this never writes back
/// to the model).
fn show(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        let value = value.to_string();
        input.update(cx, |i, cx| i.set_value(value, window, cx));
    }
}

fn synced_of(m: &radar_core::model::RadarModel) -> Synced {
    Synced {
        name: m.profile.name.clone(),
        role: m.profile.role.clone(),
        time: m.prefs.schedule_time.clone(),
    }
}

impl Form {
    fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let synced = synced_of(&AppModel::global(cx).read(cx).model);
        let name = new_input("e.g. Maya", &synced.name, window, cx);
        let role = new_input("e.g. Freelance · Branding", &synced.role, window, cx);
        let time = parse_time(&synced.time);
        let schedule = cx.new(|cx| {
            let mut state = TimeFieldState::new(window, cx);
            if let Some(t) = time {
                state.set_time(t, window, cx);
            }
            state
        });
        let on_text = |field: Field| {
            move |ws: &mut Workspace,
                  input: &Entity<InputState>,
                  ev: &InputEvent,
                  _: &mut Window,
                  cx: &mut Context<Workspace>| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                if let Some(f) = ws.settings.form() {
                    match field {
                        Field::Name => f.synced.name = value.clone(),
                        Field::Role => f.synced.role = value.clone(),
                    }
                }
                AppModel::mutate(cx, |m| match field {
                    Field::Name => m.profile.name = value,
                    Field::Role => m.profile.role = value,
                });
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&name, window, on_text(Field::Name)),
            cx.subscribe_in(&role, window, on_text(Field::Role)),
            cx.subscribe_in(&schedule, window, |ws, _, ev: &TimeFieldEvent, _, cx| {
                let TimeFieldEvent::Change(t) = ev;
                let value = t.format("%H:%M").to_string();
                if let Some(f) = ws.settings.form() {
                    f.synced.time = value.clone();
                }
                AppModel::mutate(cx, |m| m.prefs.schedule_time = value);
            }),
        ];
        Self {
            name,
            role,
            schedule,
            lens: None,
            api: ApiKey::default(),
            autostart: autostart::is_enabled(),
            synced,
            _subscriptions: subscriptions,
        }
    }
}

impl SettingsView {
    /// Builds the form on first show (and reads the key status), then shows
    /// model changes made elsewhere in the controlled fields.
    pub fn prepare(&mut self, window: &mut Window, cx: &mut Context<Workspace>) {
        let Some(form) = self.form.as_mut() else {
            self.form = Some(Form::new(window, cx));
            self.refresh_key(cx);
            return;
        };
        let now = synced_of(&AppModel::global(cx).read(cx).model);
        if now == form.synced {
            return;
        }
        if now.name != form.synced.name {
            show(&form.name, &now.name, window, cx);
        }
        if now.role != form.synced.role {
            show(&form.role, &now.role, window, cx);
        }
        if let Some(t) = parse_time(&now.time).filter(|_| now.time != form.synced.time) {
            form.schedule.update(cx, |s, cx| s.set_time(t, window, cx));
        }
        form.synced = now;
    }

    fn form(&mut self) -> Option<&mut Form> {
        self.form.as_mut()
    }

    /// `refresh()`: reads `{present, preview}` from the keychain.
    fn refresh_key(&mut self, cx: &mut Context<Workspace>) {
        let store = AppModel::global(cx).read(cx).key_store.clone();
        cx.spawn(async move |ws, cx| {
            let status =
                bridge::on_blocking(move || keychain::anthropic_api_key_status(&*store)).await;
            let _ = ws.update(cx, |ws, cx| {
                if let Some(f) = ws.settings.form() {
                    match status {
                        Ok(st) => {
                            f.api.present = st.present;
                            f.api.preview = st.preview;
                        }
                        Err(e) => f.api.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn key_edit(&mut self, window: &mut Window, cx: &mut Context<Workspace>) {
        let draft = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("sk-ant-…")
        });
        if let Some(f) = self.form() {
            f.api.draft = Some(draft);
            f.api.error = None;
        }
        cx.notify();
    }

    fn key_cancel(&mut self, cx: &mut Context<Workspace>) {
        if let Some(f) = self.form() {
            f.api.draft = None;
            f.api.error = None;
        }
        cx.notify();
    }

    /// `save()`: the key goes to the keychain (blocking pool) and only the
    /// resulting status comes back.
    fn key_save(&mut self, cx: &mut Context<Workspace>) {
        let Some(f) = self.form.as_mut() else {
            return;
        };
        let Some(draft) = f.api.draft.as_ref() else {
            return;
        };
        let key = draft.read(cx).value().to_string();
        if f.api.busy || key.trim().is_empty() {
            return;
        }
        f.api.busy = true;
        f.api.error = None;
        cx.notify();
        let store = AppModel::global(cx).read(cx).key_store.clone();
        cx.spawn(async move |ws, cx| {
            let status = bridge::on_blocking(move || {
                keychain::set_anthropic_api_key(&*store, key)?;
                keychain::anthropic_api_key_status(&*store)
            })
            .await;
            let _ = ws.update(cx, |ws, cx| {
                if let Some(f) = ws.settings.form() {
                    f.api.busy = false;
                    match status {
                        Ok(st) => {
                            f.api.draft = None;
                            f.api.present = st.present;
                            f.api.preview = st.preview;
                        }
                        Err(e) => f.api.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn key_clear(&mut self, cx: &mut Context<Workspace>) {
        let Some(f) = self.form.as_mut().filter(|f| !f.api.busy) else {
            return;
        };
        f.api.busy = true;
        f.api.error = None;
        cx.notify();
        let store = AppModel::global(cx).read(cx).key_store.clone();
        cx.spawn(async move |ws, cx| {
            let status = bridge::on_blocking(move || {
                keychain::clear_anthropic_api_key(&*store)?;
                keychain::anthropic_api_key_status(&*store)
            })
            .await;
            let _ = ws.update(cx, |ws, cx| {
                if let Some(f) = ws.settings.form() {
                    f.api.busy = false;
                    match status {
                        Ok(st) => {
                            f.api.present = st.present;
                            f.api.preview = st.preview;
                        }
                        Err(e) => f.api.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn lens_edit(&mut self, window: &mut Window, cx: &mut Context<Workspace>) {
        let lens = AppModel::global(cx).read(cx).model.profile.lens.clone();
        let state = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx)
                .rows(4)
                .placeholder(LENS_PLACEHOLDER);
            s.set_value(lens, window, cx);
            s
        });
        state.update(cx, |s, cx| s.focus(window, cx));
        if let Some(f) = self.form() {
            f.lens = Some(state);
        }
        cx.notify();
    }

    fn lens_close(&mut self, value: Option<String>, cx: &mut Context<Workspace>) {
        if let Some(f) = self.form() {
            f.lens = None;
        }
        if let Some(v) = value {
            AppModel::mutate(cx, |m| m.profile.lens = v);
        }
        cx.notify();
    }

    fn toggle_autostart(&mut self, cx: &mut Context<Workspace>) {
        if let Some(f) = self.form() {
            if let Err(e) = autostart::set(!f.autostart) {
                log::warn!("[autostart] toggle failed: {e}");
            }
            f.autostart = autostart::is_enabled();
        }
        cx.notify();
    }
}

// ---- pieces ----------------------------------------------------------------

/// A text element findable by `id` whose accessible label is its text.
fn labeled(id: String, text: SharedString) -> impl Styled + IntoElement {
    div()
        .id(SharedString::from(id))
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

/// `blockHead`: surface-2 header with the 13/500 title and the 12 px sub.
fn head(i: usize, title: &'static str, sub: &'static str, t: &Tokens) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(18.))
        .py(px(14.))
        .border_b_1()
        .border_color(t.border)
        .bg(t.surface_2)
        .child(
            labeled(format!("settings-section-{i}"), title.into())
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM),
        )
        .child(
            labeled(format!("settings-section-{i}-sub"), sub.into())
                .text_size(px(12.))
                .text_color(t.text_3),
        )
}

fn section(
    i: usize,
    title: &'static str,
    sub: &'static str,
    rows: Vec<AnyElement>,
    t: &Tokens,
) -> AnyElement {
    block(t)
        .child(head(i, title, sub, t))
        .children(rows)
        .into_any_element()
}

/// `row(label, sub, control)` / `settingRow`.
fn row(
    key: &'static str,
    label: impl Into<SharedString>,
    sub: Option<&'static str>,
    control: impl IntoElement,
    t: &Tokens,
) -> AnyElement {
    div()
        .flex()
        .justify_between()
        .items_center()
        .px(px(18.))
        .py(px(14.))
        .border_b_1()
        .border_color(t.border)
        .text_size(px(13.5))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    labeled(format!("settings-row-{key}-label"), label.into())
                        .font_weight(FontWeight::MEDIUM),
                )
                .children(sub.map(|s| {
                    labeled(format!("settings-row-{key}-sub"), s.into())
                        .mt(px(2.))
                        .text_size(px(12.))
                        .text_color(t.text_3)
                })),
        )
        .child(control)
        .into_any_element()
}

/// JetBrains Mono 12 px text (the "—", "Opus 4.7", version, preview spans).
fn mono(id: &'static str, text: impl Into<SharedString>, color: Hsla) -> Div {
    let text: SharedString = text.into();
    div().child(
        div()
            .id(id)
            .test_support()
            .aria_label(text.clone())
            .font_family(theme::FONT_MONO)
            .text_size(px(12.))
            .text_color(color)
            .child(text),
    )
}

fn controls(gap: f32) -> Div {
    div().flex().flex_none().items_center().gap(px(gap))
}

fn avatar(account: &Account, t: &Tokens) -> AnyElement {
    let source = if account.name.trim().is_empty() {
        &account.email
    } else {
        &account.name
    };
    let initial: SharedString = source
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".into())
        .into();
    let (soft, text) = (t.accent_soft, t.accent_text);
    let initial_avatar = move || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded_full()
            .bg(soft)
            .text_color(text)
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial.clone())
            .into_any_element()
    };
    match account.image_url.clone() {
        Some(url) => img(url)
            .flex_none()
            .size(px(28.))
            .rounded_full()
            .bg(t.surface_2)
            .object_fit(ObjectFit::Cover)
            .with_fallback(initial_avatar)
            .into_any_element(),
        None => initial_avatar(),
    }
}

/// `AccountControl`: sign-in pill, or who is signed in plus Sign out.
fn account_control(app: &AppModel, t: &Tokens) -> AnyElement {
    if let Some(account) = &app.account {
        let who = div()
            .flex()
            .flex_col()
            .items_end()
            .min_w_0()
            .when(!account.name.trim().is_empty(), |d| {
                d.child(
                    labeled("settings-account-name".into(), account.name.clone().into())
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM),
                )
            })
            .child(
                labeled(
                    "settings-account-email".into(),
                    account.email.clone().into(),
                )
                .font_family(theme::FONT_MONO)
                .text_size(px(11.5))
                .text_color(t.text_3),
            );
        let sign_out = Pill::new("settings-sign-out", "Sign out")
            .on_click(|_, _, cx| AppModel::global(cx).update(cx, |m, cx| m.sign_out(cx)))
            .render(t, []);
        return controls(10.)
            .child(who)
            .child(avatar(account, t))
            .child(sign_out)
            .into_any_element();
    }
    if app.signing_in {
        // The browser flow is running (JSX: the "Loading auth…" state).
        return div()
            .child(
                labeled("settings-signing-in".into(), "Loading auth…".into())
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.5))
                    .text_color(t.text_3),
            )
            .into_any_element();
    }
    let pill = Pill::new("settings-sign-in", "Sign in");
    if app.session.is_none() {
        let hint = "Clerk not configured";
        return controls(10.)
            .child(
                labeled("settings-sign-in-hint".into(), hint.into())
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.5))
                    .text_color(t.text_3),
            )
            .child(
                pill.title(Some(
                    "Set RADAR_CLERK_FRONTEND_API and RADAR_CLERK_CLIENT_ID".into(),
                ))
                .dimmed()
                .render(t, []),
            )
            .into_any_element();
    }
    pill.on_click(|_, _, cx| AppModel::global(cx).update(cx, |m, cx| m.sign_in(cx)))
        .render(t, [])
        .into_any_element()
}

fn text_field(
    id: &'static str,
    label: &'static str,
    state: &Entity<InputState>,
    t: &Tokens,
) -> Div {
    div()
        .flex_none()
        .w(px(240.))
        .child(text_input(id, label, state, false, t))
}

fn schedule_control(f: &Form, enabled: bool, t: &Tokens) -> Div {
    let field = TimeField::new(&f.schedule)
        .with_size(Size::Size(px(12. / 0.875)))
        .disabled(!enabled)
        .h(px(28.))
        .px(px(8.))
        .rounded(t.radius)
        .border_color(t.border)
        .bg(t.surface)
        .text_color(t.text)
        .font_family(theme::FONT_MONO)
        .text_size(px(12.));
    controls(8.).child(field).child(
        Toggle::new("settings-schedule-toggle", enabled)
            .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.schedule_enabled ^= true)),
    )
}

fn lens_control(f: &Form, lens: &str, t: &Tokens, cx: &Context<Workspace>) -> AnyElement {
    let Some(state) = &f.lens else {
        let label = if lens.trim().is_empty() {
            "Add"
        } else {
            "Edit"
        };
        return Pill::new("settings-lens-edit", label)
            .on_click(cx.listener(|ws, _, window, cx| ws.settings.lens_edit(window, cx)))
            .render(t, [])
            .into_any_element();
    };
    let editor = Textarea::new(state)
        .with_size(Size::Size(px(13. / 0.875)))
        .aria_label("Professional lens")
        .w_full()
        // CSS `padding: 8px 10px`, less the kit's own multi-line editor
        // padding (8 px horizontal, 2 px vertical).
        .px(px(2.))
        .py(px(6.))
        .rounded(t.radius)
        .border_color(t.border)
        .bg(t.surface)
        .text_color(t.text)
        .font_family(theme::FONT_SANS)
        .text_size(px(13.))
        // A `<textarea>` keeps the UA `line-height: normal` (Inter: ~1.21).
        .line_height(px(LENS_LINE_HEIGHT));
    let draft = state.clone();
    let save = Pill::new("settings-lens-save", "Save")
        .on_click(cx.listener(move |ws, _, _, cx| {
            let value = draft.read(cx).value().to_string();
            ws.settings.lens_close(Some(value), cx);
        }))
        .render(t, []);
    let cancel = Pill::new("settings-lens-cancel", "Cancel")
        .on_click(cx.listener(|ws, _, _, cx| ws.settings.lens_close(None, cx)))
        .render(t, []);
    let clear = (!lens.trim().is_empty()).then(|| {
        Pill::new("settings-lens-clear", "Clear")
            .style(PillStyle::Danger)
            .on_click(cx.listener(|ws, _, _, cx| ws.settings.lens_close(Some(String::new()), cx)))
            .render(t, [])
    });
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(8.))
        .w(px(LENS_EDITOR_WIDTH))
        .child(
            div()
                .id("settings-lens-input")
                .test_support()
                .w_full()
                .child(editor),
        )
        .child(controls(6.).child(save).child(cancel).children(clear))
        .into_any_element()
}

fn api_key_control(api: &ApiKey, t: &Tokens, cx: &Context<Workspace>) -> AnyElement {
    let rust = t.dot_for("rust").unwrap_or(t.text_2);
    if let Some(draft) = &api.draft {
        let input = text_input(
            "settings-api-key-input",
            "Anthropic API key",
            draft,
            true,
            t,
        );
        let has_draft = !draft.read(cx).value().trim().is_empty();
        let save = Pill::new(
            "settings-api-key-save",
            if api.busy { "Saving…" } else { "Save" },
        )
        .on_click(cx.listener(|ws, _, _, cx| ws.settings.key_save(cx)))
        .enabled(!api.busy && has_draft)
        .render(t, []);
        let cancel = Pill::new("settings-api-key-cancel", "Cancel")
            .on_click(cx.listener(|ws, _, _, cx| ws.settings.key_cancel(cx)))
            .render(t, []);
        let error = api.error.clone().map(|e| {
            labeled("settings-api-key-error".into(), e.into())
                .text_size(px(11.))
                .text_color(rust)
        });
        return controls(6.)
            .child(div().flex_none().w(px(220.)).child(input))
            .child(save)
            .child(cancel)
            .children(error)
            .into_any_element();
    }
    if api.present {
        let preview = api.preview.clone().unwrap_or_else(|| "••••••".into());
        let replace = Pill::new("settings-api-key-replace", "Replace")
            .on_click(cx.listener(|ws, _, window, cx| ws.settings.key_edit(window, cx)))
            .render(t, []);
        let clear = Pill::new("settings-api-key-clear", "Clear")
            .on_click(cx.listener(|ws, _, _, cx| ws.settings.key_clear(cx)))
            .enabled(!api.busy)
            .render(t, []);
        return controls(6.)
            .child(mono("settings-api-key-preview", preview, t.text_2))
            .child(replace)
            .child(clear)
            .into_any_element();
    }
    Pill::new("settings-api-key-add", "Add key")
        .on_click(cx.listener(|ws, _, window, cx| ws.settings.key_edit(window, cx)))
        .render(t, [])
        .into_any_element()
}

fn choice<T: Copy + PartialEq + 'static>(
    id: &'static str,
    label: &'static str,
    icon: Option<IconName>,
    value: T,
    current: T,
    set: fn(&mut radar_core::types::Prefs, T),
    t: &Tokens,
) -> impl IntoElement {
    let pill = Pill::new(id, label)
        .style(if value == current {
            PillStyle::Active
        } else {
            PillStyle::Plain
        })
        .on_click(move |_, _, cx| AppModel::mutate(cx, |m| set(&mut m.prefs, value)));
    match icon {
        Some(icon) => pill.icon(icon).icon_size(12.),
        None => pill,
    }
    .render(t, [])
}

fn updates_control(t: &Tokens) -> AnyElement {
    // `updater::State::Idle`: version meta plus "Check for updates".
    let updater::State::Idle = updater::state();
    let meta = format!("v{}", env!("CARGO_PKG_VERSION"));
    controls(10.)
        .child(
            mono("settings-updates-meta", meta, t.text_3).font_features(theme::font_features(true)),
        )
        .child(
            Pill::new("settings-updates-check", "Check for updates")
                .on_click(|_, _, _| updater::check())
                .render(t, []),
        )
        .into_any_element()
}

pub fn render(view: &SettingsView, app: &AppModel, cx: &Context<Workspace>) -> Div {
    let t = *theme::t(cx);
    let m = &app.model;
    let prefs = &m.prefs;
    let signed_in = app.account.is_some();

    let account = section(
        0,
        "Account",
        "Enables cross-device sync",
        vec![row(
            "account",
            if signed_in { "Signed in" } else { "Sign in" },
            Some(if signed_in {
                "Signed in via Clerk. Sync runs automatically."
            } else {
                "Sync is off. You'll stay signed out and local-only until you connect."
            }),
            account_control(app, &t),
            &t,
        )],
        &t,
    );

    let mut sections = vec![account];
    if let Some(f) = &view.form {
        sections.push(section(
            1,
            "Profile",
            "Shown in the greeting and sidebar",
            vec![
                row(
                    "name",
                    "Name",
                    Some("How Radar addresses you."),
                    text_field("settings-profile-name", "Name", &f.name, &t),
                    &t,
                ),
                row(
                    "role",
                    "Role",
                    Some("Short tagline under your name."),
                    text_field("settings-profile-role", "Role", &f.role, &t),
                    &t,
                ),
            ],
            &t,
        ));
        sections.push(section(
            2,
            "Briefing",
            "Controls when and how Radar scans",
            vec![
                row(
                    "run-at-launch",
                    "Run at launch",
                    Some("Trigger a fresh Morning Briefing every time you open Radar."),
                    Toggle::new("settings-run-at-launch", prefs.run_at_launch).on_click(
                        |_, _, cx| AppModel::mutate(cx, |m| m.prefs.run_at_launch ^= true),
                    ),
                    &t,
                ),
                row(
                    "schedule",
                    "Schedule",
                    Some(
                        "Fire a briefing at this local time each day. Runs only while Radar is \
                         open; missed slots fire on next launch.",
                    ),
                    schedule_control(f, prefs.schedule_enabled, &t),
                    &t,
                ),
                row(
                    "autostart",
                    "Open at login",
                    Some(
                        "Launch Radar hidden to the menu bar when you boot your Mac — keeps \
                         scheduled briefings firing without re-opening.",
                    ),
                    Toggle::new("settings-autostart", f.autostart)
                        .on_click(cx.listener(|ws, _, _, cx| ws.settings.toggle_autostart(cx))),
                    &t,
                ),
                row(
                    "staleness",
                    "Staleness threshold",
                    Some("Not wired — articles aren't cached across briefings yet."),
                    mono("settings-staleness", "—", t.text_3),
                    &t,
                ),
            ],
            &t,
        ));
        sections.push(section(
            3,
            "Curation",
            "Anthropic Claude filters and summarizes",
            vec![
                row(
                    "model",
                    "Model",
                    Some("claude-opus-4-7"),
                    mono("settings-model", "Opus 4.7", t.text_2),
                    &t,
                ),
                row(
                    "lens",
                    "Professional lens",
                    Some("Extra context passed to Claude so it leans into what you care about."),
                    lens_control(f, &m.profile.lens, &t, cx),
                    &t,
                ),
                row(
                    "api-key",
                    "API key",
                    Some("Stored in the system keychain. Sent only to api.anthropic.com."),
                    api_key_control(&f.api, &t, cx),
                    &t,
                ),
            ],
            &t,
        ));
    }
    sections.push(section(
        4,
        "Appearance",
        "Local preferences",
        vec![
            row(
                "theme",
                "Theme",
                None,
                controls(4.)
                    .child(choice(
                        "settings-theme-light",
                        "Light",
                        Some(IconName::Sun),
                        Theme::Light,
                        prefs.theme,
                        |p, v| p.theme = v,
                        &t,
                    ))
                    .child(choice(
                        "settings-theme-dark",
                        "Dark",
                        Some(IconName::Moon),
                        Theme::Dark,
                        prefs.theme,
                        |p, v| p.theme = v,
                        &t,
                    )),
                &t,
            ),
            row(
                "show-accent",
                "Accent bars on cards",
                Some("Show a colored edge indicating category."),
                Toggle::new("settings-show-accent", prefs.show_accent)
                    .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.show_accent ^= true)),
                &t,
            ),
            row(
                "use-serif",
                "Serif headlines",
                Some("Use Newsreader for article titles."),
                Toggle::new("settings-use-serif", prefs.use_serif)
                    .on_click(|_, _, cx| AppModel::mutate(cx, |m| m.prefs.use_serif ^= true)),
                &t,
            ),
            row(
                "density",
                "Density",
                Some("Compact packs more rows in list view."),
                controls(4.)
                    .child(choice(
                        "settings-density-cozy",
                        "Cozy",
                        None,
                        Density::Cozy,
                        prefs.density,
                        |p, v| p.density = v,
                        &t,
                    ))
                    .child(choice(
                        "settings-density-compact",
                        "Compact",
                        None,
                        Density::Compact,
                        prefs.density,
                        |p, v| p.density = v,
                        &t,
                    )),
                &t,
            ),
        ],
        &t,
    ));
    sections.push(section(
        5,
        "Updates",
        "Signed releases from GitHub",
        vec![row(
            "updates",
            "App updates",
            Some(
                "Radar checks on launch and when you ask. Downloads are signed and verified \
                 before install.",
            ),
            updates_control(&t),
            &t,
        )],
        &t,
    ));
    sections.push(section(
        6,
        "Integrations",
        "Not yet shipped",
        vec![row(
            "gmail",
            "Gmail — newsletter inbox",
            Some(
                "Read-only, label-scoped. Requires a Google OAuth client ID which isn't \
                 bundled yet.",
            ),
            Pill::new("settings-gmail-connect", "Connect")
                .dimmed()
                .render(&t, []),
            &t,
        )],
        &t,
    ));

    let mut children = vec![view_subtitle("settings-subtitle", SUBTITLE, &t).into_any_element()];
    children.extend(sections);
    page_frame("Settings", children).max_w(px(MAX_WIDTH))
}
