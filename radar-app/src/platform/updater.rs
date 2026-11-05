//! App updates through `cargo-packager-updater`.
//!
//! The manifest is the `latest.json` of the GitHub release
//! ([`ENDPOINT`]); artifacts are verified against the minisign public key
//! earlier releases already ship ([`PUBKEY`]), so installed older builds and
//! this app trust the same signatures. See `scripts/make-latest-json.sh` for
//! the manifest shape.
//!
//! # States
//! [`State`] is the update state machine. [`AppModel::check_for_updates`],
//! [`download_update`](AppModel::download_update) and
//! [`restart_to_apply`](AppModel::restart_to_apply) drive it; all network and
//! install work runs on the blocking pool through a [`Backend`], which tests
//! replace (`Hooks::updater`). The launch check is silent: finding nothing
//! or failing leaves the state `Idle` instead of showing "up to date" or an
//! error nobody asked for.
//!
//! # Install
//! The model is flushed to disk between download and install, because on
//! Windows the installer takes over and the process exits without the quit
//! hook running.

use crate::app_model::AppModel;
use crate::bridge;
use cargo_packager_updater::semver::Version;
use cargo_packager_updater::{Config, Update, UpdaterBuilder};
use gpui_kit::{App, Context};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;

/// Release manifest; `latest` resolves to the newest published release.
pub const ENDPOINT: &str =
    "https://github.com/alamanija/radar/releases/latest/download/latest.json";
/// The release-signing minisign public key, unchanged since earlier
/// releases (installed older builds verify updates against it too).
pub const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEM2N0JFNUNDOEVDRUQwOTMKUldTVDBNNk96T1Y3eGs1ODJHVkxjdko0RCtDVmdFT1FGUnNCR1FoMENzaW51Nk5MRDVQVno3QysK";

/// Where the update flow stands.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum State {
    /// Nothing in flight.
    #[default]
    Idle,
    Checking,
    /// A newer version exists; the user chooses when to download.
    Available {
        version: String,
        notes: String,
    },
    /// `progress` is 0..=1, or `None` while the size is unknown.
    Downloading {
        progress: Option<f32>,
    },
    /// Installed; a restart applies it.
    Ready,
    /// Set by a user-initiated check only.
    UpToDate,
    Error(String),
}

/// A newer release found by [`Backend::check`].
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
}

/// The update client. Every method blocks; the model calls them on the
/// blocking pool.
pub trait Backend: Send + Sync {
    /// `Ok(None)` when already up to date.
    fn check(&self) -> Result<Option<Release>, String>;
    /// Downloads and verifies the update found by the last `check`,
    /// reporting progress as a 0..=1 fraction.
    fn download(&self, progress: &(dyn Fn(f32) + Send + Sync)) -> Result<(), String>;
    /// Installs what `download` fetched. On Windows the installer takes over
    /// and this does not return.
    fn install(&self) -> Result<(), String>;
    /// Starts a fresh copy once this process has exited.
    fn restart(&self) -> Result<(), String>;
}

/// Finds nothing and does nothing (tests and `Hooks::inert`).
pub struct Inert;

impl Backend for Inert {
    fn check(&self) -> Result<Option<Release>, String> {
        Ok(None)
    }
    fn download(&self, _: &(dyn Fn(f32) + Send + Sync)) -> Result<(), String> {
        Ok(())
    }
    fn install(&self) -> Result<(), String> {
        Ok(())
    }
    fn restart(&self) -> Result<(), String> {
        Ok(())
    }
}

/// `cargo-packager-updater` against a manifest URL.
pub struct Packager {
    updater: cargo_packager_updater::Updater,
    pending: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
}

impl Packager {
    /// Manifest key of this platform. The manifest names macOS `darwin` (the
    /// key older installed builds' updater reads); the packager crate's own
    /// spelling is `macos`.
    pub fn platform_key() -> String {
        cargo_packager_updater::target()
            .map(|t| t.replacen("macos-", "darwin-", 1))
            .unwrap_or_else(|| "unsupported".into())
    }

    pub fn new(endpoint: &str, pubkey: &str, current: &str) -> Result<Self, String> {
        let config = Config {
            endpoints: vec![endpoint.parse().map_err(|e| format!("endpoint: {e}"))?],
            pubkey: pubkey.into(),
            ..Config::default()
        };
        let current = Version::parse(current).map_err(|e| format!("version: {e}"))?;
        let updater = UpdaterBuilder::new(current, config)
            .target(Self::platform_key())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            updater,
            pending: Mutex::default(),
        })
    }

    /// The release endpoint and key, for the running version.
    pub fn production() -> Result<Self, String> {
        Self::new(ENDPOINT, PUBKEY, env!("CARGO_PKG_VERSION"))
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Backend for Packager {
    fn check(&self) -> Result<Option<Release>, String> {
        let found = self.updater.check().map_err(|e| e.to_string())?;
        let release = found.as_ref().map(|u| Release {
            version: u.version.clone(),
            notes: u.body.clone().unwrap_or_default(),
        });
        *self.pending() = Pending {
            update: found,
            bytes: None,
        };
        Ok(release)
    }

    fn download(&self, progress: &(dyn Fn(f32) + Send + Sync)) -> Result<(), String> {
        let update = self
            .pending()
            .update
            .clone()
            .ok_or("no update to download")?;
        let received = AtomicU64::new(0);
        let last_percent = AtomicU64::new(0);
        let bytes = update
            .download_extended(
                |chunk, total| {
                    let got = received.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
                    let Some(total) = total.filter(|t| *t > 0) else {
                        return;
                    };
                    let fraction = (got as f64 / total as f64).min(1.0);
                    // One report per whole percent, not per chunk.
                    let percent = (fraction * 100.0) as u64;
                    if last_percent.swap(percent, Ordering::Relaxed) != percent {
                        progress(fraction as f32);
                    }
                },
                || progress(1.0),
            )
            .map_err(|e| e.to_string())?;
        self.pending().bytes = Some(bytes);
        Ok(())
    }

    fn install(&self) -> Result<(), String> {
        let (update, bytes) = {
            let mut p = self.pending();
            (p.update.clone(), p.bytes.take())
        };
        match (update, bytes) {
            (Some(update), Some(bytes)) => update.install(bytes).map_err(|e| e.to_string()),
            _ => Err("no downloaded update to install".into()),
        }
    }

    fn restart(&self) -> Result<(), String> {
        relaunch_after_exit()
    }
}

/// Starts the app again once this process is gone. A new instance launched
/// while this one is still alive would only ask it to show its window and
/// exit, so the helper waits for this process first.
#[cfg(unix)]
fn relaunch_after_exit() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$0""#;
    std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg(exe)
        .arg(std::process::id().to_string())
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn relaunch_after_exit() -> Result<(), String> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new("cmd")
        .creation_flags(CREATE_NO_WINDOW)
        .raw_arg(format!(
            "/C ping -n 4 127.0.0.1 >NUL & start \"\" \"{}\"",
            exe.display()
        ))
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// The silent check at startup. Never blocks it: the work is off-thread.
pub fn check_on_launch(cx: &mut App) {
    AppModel::global(cx).update(cx, |m, cx| m.check_for_updates(true, cx));
}

impl AppModel {
    /// "Check for updates" (`silent: false`) or the launch check. Ignored
    /// while a check or download is running.
    pub fn check_for_updates(&mut self, silent: bool, cx: &mut Context<Self>) {
        if matches!(self.update, State::Checking | State::Downloading { .. }) {
            return;
        }
        self.update = State::Checking;
        cx.notify();
        let backend = self.hooks.updater.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge::on_blocking(move || backend.check()).await;
            let _ = this.update(cx, |m, cx| {
                m.update = match result {
                    Ok(Some(r)) => State::Available {
                        version: r.version,
                        notes: r.notes,
                    },
                    Ok(None) if silent => State::Idle,
                    Ok(None) => State::UpToDate,
                    Err(e) => {
                        log::warn!("[updater] check failed: {e}");
                        if silent {
                            State::Idle
                        } else {
                            State::Error(e)
                        }
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// "Download": fetch and verify, flush the model, install, then `Ready`.
    /// With nothing found yet, it checks first instead.
    pub fn download_update(&mut self, cx: &mut Context<Self>) {
        match self.update {
            State::Available { .. } => {}
            State::Idle | State::UpToDate | State::Error(_) => {
                return self.check_for_updates(false, cx)
            }
            _ => return,
        }
        self.update = State::Downloading { progress: None };
        cx.notify();
        let backend = self.hooks.updater.clone();
        let (tx, rx) = async_channel::unbounded::<f32>();
        let download = {
            let backend = backend.clone();
            bridge::on_blocking(move || {
                backend.download(&move |p| {
                    let _ = tx.try_send(p);
                })
            })
        };
        cx.spawn(async move |this, cx| {
            // Ends when the download closure, which owns `tx`, is dropped.
            while let Ok(p) = rx.recv().await {
                let _ = this.update(cx, |m, cx| {
                    if let State::Downloading { .. } = m.update {
                        m.update = State::Downloading { progress: Some(p) };
                        cx.notify();
                    }
                });
            }
            let mut result = download.await;
            if result.is_ok() {
                if let Ok(flushed) = this.update(cx, |m, cx| m.flush_now(cx)) {
                    flushed.await;
                }
                result = bridge::on_blocking(move || backend.install()).await;
            }
            let _ = this.update(cx, |m, cx| {
                m.update = match result {
                    Ok(()) => State::Ready,
                    Err(e) => {
                        log::error!("[updater] download/install failed: {e}");
                        State::Error(e)
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// "Restart now": starts the relaunch helper, then quits (flushing as any
    /// quit does). A failed relaunch keeps the app running.
    pub fn restart_to_apply(&mut self, cx: &mut Context<Self>) {
        if self.update != State::Ready {
            return;
        }
        let backend = self.hooks.updater.clone();
        let quit = self.hooks.quit.clone();
        cx.spawn(async move |this, cx| {
            match bridge::on_blocking(move || backend.restart()).await {
                Ok(()) => cx.update(|cx| quit(cx)),
                Err(e) => {
                    log::error!("[updater] restart failed: {e}");
                    let _ = this.update(cx, |m, cx| {
                        m.update = State::Error(e);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

/// Scripted [`Backend`] for tests: records calls, answers what it is told.
/// `(version, notes)` of the release a [`Fake`] finds.
#[cfg(test)]
type Found = Result<Option<(String, String)>, String>;

#[cfg(test)]
#[derive(Clone)]
pub struct Fake {
    pub found: Arc<Mutex<Found>>,
    pub download: Arc<Mutex<Result<(), String>>>,
    pub install: Arc<Mutex<Result<(), String>>>,
    pub restart: Arc<Mutex<Result<(), String>>>,
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[cfg(test)]
impl Default for Fake {
    fn default() -> Self {
        Self {
            found: Arc::new(Mutex::new(Ok(None))),
            download: Arc::new(Mutex::new(Ok(()))),
            install: Arc::new(Mutex::new(Ok(()))),
            restart: Arc::new(Mutex::new(Ok(()))),
            log: Arc::default(),
        }
    }
}

#[cfg(test)]
impl Fake {
    pub fn calls(&self) -> Vec<&'static str> {
        self.log.lock().unwrap().clone()
    }

    fn record<T: Clone>(&self, call: &'static str, answer: &Mutex<T>) -> T {
        self.log.lock().unwrap().push(call);
        answer.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Backend for Fake {
    fn check(&self) -> Result<Option<Release>, String> {
        let found = self.record("check", &self.found)?;
        Ok(found.map(|(version, notes)| Release { version, notes }))
    }
    fn download(&self, progress: &(dyn Fn(f32) + Send + Sync)) -> Result<(), String> {
        progress(0.5);
        self.record("download", &self.download)
    }
    fn install(&self) -> Result<(), String> {
        self.record("install", &self.install)
    }
    fn restart(&self) -> Result<(), String> {
        self.record("restart", &self.restart)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_model::AppModel;
    use crate::test_support::wait_until;
    use gpui_kit::{Entity, TestAppContext};
    use httpmock::prelude::*;
    use serde_json::json;
    use std::path::Path;
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    const CURRENT: &str = "0.2.22";

    fn model(cx: &mut TestAppContext, dir: &tempfile::TempDir) -> Entity<AppModel> {
        cx.executor().allow_parking();
        cx.update(|cx| AppModel::init(dir.path().join("radar.json"), None, cx))
    }

    fn manifest(version: &str) -> serde_json::Value {
        let entry = json!({
            "signature": "c2ln",
            "url": "http://127.0.0.1:9/never-fetched",
            "format": "app",
        });
        json!({
            "version": version,
            "notes": "Fixes things",
            "pub_date": "2026-11-05T00:00:00Z",
            "platforms": { Packager::platform_key(): entry },
        })
    }

    fn state(model: &Entity<AppModel>, cx: &mut TestAppContext) -> State {
        model.read_with(cx, |m, _| m.update.clone())
    }

    fn check(model: &Entity<AppModel>, cx: &mut TestAppContext, silent: bool) -> State {
        model.update(cx, |m, cx| m.check_for_updates(silent, cx));
        wait_until(cx, |cx| state(model, cx) != State::Checking);
        state(model, cx)
    }

    fn use_packager(model: &Entity<AppModel>, cx: &mut TestAppContext, url: String) {
        let backend = Packager::new(&url, "pubkey", CURRENT).unwrap();
        model.update(cx, |m, _| m.hooks.updater = Arc::new(backend));
    }

    /// The real client against a mock manifest endpoint (no GitHub).
    #[gpui_kit::test]
    fn updater_state_transitions(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let model = model(cx, &dir);
        assert_eq!(state(&model, cx), State::Idle);

        let server = MockServer::start();
        let mut newer = server.mock(|when, then| {
            when.method(GET).path("/latest.json");
            then.status(200).json_body(manifest("0.3.0"));
        });
        use_packager(&model, cx, server.url("/latest.json"));
        assert_eq!(
            check(&model, cx, false),
            State::Available {
                version: "0.3.0".into(),
                notes: "Fixes things".into()
            }
        );
        newer.assert_calls(1);
        newer.delete();

        let mut same = server.mock(|when, then| {
            when.method(GET).path("/latest.json");
            then.status(200).json_body(manifest(CURRENT));
        });
        assert_eq!(check(&model, cx, false), State::UpToDate);
        same.delete();

        server.mock(|when, then| {
            when.method(GET).path("/latest.json");
            then.status(500);
        });
        assert!(matches!(check(&model, cx, false), State::Error(_)));
    }

    #[gpui_kit::test]
    fn silent_launch_check_never_shows_failure_or_up_to_date(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let model = model(cx, &dir);
        let fake = Fake::default();
        model.update(cx, |m, _| m.hooks.updater = Arc::new(fake.clone()));
        assert_eq!(check(&model, cx, true), State::Idle, "up to date, silent");

        *fake.found.lock().unwrap() = Ok(Some(("0.3.0".into(), "n".into())));
        assert!(matches!(check(&model, cx, true), State::Available { .. }));

        let fresh = Fake::default();
        *fresh.found.lock().unwrap() = Err("offline".into());
        model.update(cx, |m, _| {
            m.update = State::Idle;
            m.hooks.updater = Arc::new(fresh.clone());
        });
        assert_eq!(check(&model, cx, true), State::Idle, "failure, silent");
        assert_eq!(check(&model, cx, false), State::Error("offline".into()));
    }

    #[gpui_kit::test]
    fn download_installs_then_restart_quits(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let model = model(cx, &dir);
        let fake = Fake::default();
        *fake.found.lock().unwrap() = Ok(Some(("0.3.0".into(), String::new())));
        let quits = Arc::new(AtomicUsize::new(0));
        let q = quits.clone();
        model.update(cx, |m, _| {
            m.hooks.updater = Arc::new(fake.clone());
            m.hooks.quit = Rc::new(move |_| {
                q.fetch_add(1, SeqCst);
            });
        });
        // Nothing to download yet: the click is a re-check.
        model.update(cx, |m, cx| m.download_update(cx));
        wait_until(cx, |cx| {
            matches!(state(&model, cx), State::Available { .. })
        });
        assert_eq!(fake.calls(), ["check"]);

        model.update(cx, |m, cx| m.download_update(cx));
        wait_until(cx, |cx| state(&model, cx) == State::Ready);
        assert_eq!(fake.calls(), ["check", "download", "install"]);

        model.update(cx, |m, cx| m.restart_to_apply(cx));
        wait_until(cx, |_| quits.load(SeqCst) == 1);
        assert_eq!(fake.calls().last(), Some(&"restart"));
    }

    #[gpui_kit::test]
    fn failed_download_and_failed_restart_report_errors(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let model = model(cx, &dir);
        let fake = Fake::default();
        *fake.found.lock().unwrap() = Ok(Some(("0.3.0".into(), String::new())));
        *fake.download.lock().unwrap() = Err("bad signature".into());
        let quits = Arc::new(AtomicUsize::new(0));
        let q = quits.clone();
        model.update(cx, |m, _| {
            m.hooks.updater = Arc::new(fake.clone());
            m.hooks.quit = Rc::new(move |_| {
                q.fetch_add(1, SeqCst);
            });
        });
        check(&model, cx, false);
        model.update(cx, |m, cx| m.download_update(cx));
        wait_until(cx, |cx| matches!(state(&model, cx), State::Error(_)));
        assert_eq!(state(&model, cx), State::Error("bad signature".into()));
        assert!(
            !fake.calls().contains(&"install"),
            "never installs unverified"
        );

        // Restart outside `Ready` does nothing.
        model.update(cx, |m, cx| m.restart_to_apply(cx));
        cx.run_until_parked();
        assert_eq!(quits.load(SeqCst), 0);
        assert!(!fake.calls().contains(&"restart"));

        model.update(cx, |m, _| m.update = State::Ready);
        *fake.restart.lock().unwrap() = Err("no exe".into());
        model.update(cx, |m, cx| m.restart_to_apply(cx));
        wait_until(cx, |cx| matches!(state(&model, cx), State::Error(_)));
        assert_eq!(quits.load(SeqCst), 0, "a failed relaunch keeps the app up");
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn make_latest(dir: &Path, extra_env: &[(&str, &str)]) -> std::process::Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/make-latest-json.sh");
        Command::new("bash")
            .arg(script)
            .args(["0.3.0", "alamanija/radar"])
            .arg(dir)
            .env("RELEASE_NOTES", "Notes \"quoted\"")
            .env("PUB_DATE", "2026-11-05T00:00:00Z")
            .envs(extra_env.iter().copied())
            .output()
            .unwrap()
    }

    /// `latest.json` must stay readable by updaters in already-installed
    /// older builds (`version`, `notes`, `pub_date`,
    /// `platforms.<os>-<arch>.{signature,url}`) and by cargo-packager-updater
    /// (which also wants `format`).
    #[test]
    fn latest_json_shape() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Radar_0.3.0_x64-setup.exe", "exe");
        write(dir.path(), "Radar_0.3.0_x64-setup.exe.sig", "WINSIG\n");
        write(dir.path(), "Radar_0.3.0_aarch64.app.tar.gz", "tgz");
        write(dir.path(), "Radar_0.3.0_aarch64.app.tar.gz.sig", "MACSIG\n");
        write(dir.path(), "Radar_0.3.0_aarch64.dmg", "dmg");

        let out = make_latest(dir.path(), &[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["version"], "0.3.0");
        assert_eq!(v["notes"], "Notes \"quoted\"");
        assert_eq!(v["pub_date"], "2026-11-05T00:00:00Z");
        let base = "https://github.com/alamanija/radar/releases/download/v0.3.0";
        let win = &v["platforms"]["windows-x86_64"];
        assert_eq!(win["signature"], "WINSIG");
        assert_eq!(win["url"], format!("{base}/Radar_0.3.0_x64-setup.exe"));
        assert_eq!(win["format"], "nsis");
        let mac = &v["platforms"]["darwin-aarch64"];
        assert_eq!(mac["signature"], "MACSIG");
        assert_eq!(mac["url"], format!("{base}/Radar_0.3.0_aarch64.app.tar.gz"));
        assert_eq!(mac["format"], "app");
        assert_eq!(v["platforms"].as_object().unwrap().len(), 2);

        // cargo-packager-updater parses it as a release.
        let parsed: cargo_packager_updater::RemoteRelease =
            serde_json::from_value(v).expect("updater reads the manifest");
        assert_eq!(parsed.version.to_string(), "0.3.0");
    }

    #[test]
    fn latest_json_refuses_a_half_release() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Radar_0.3.0_x64-setup.exe", "exe");
        write(dir.path(), "Radar_0.3.0_x64-setup.exe.sig", "WINSIG\n");
        let out = make_latest(dir.path(), &[]);
        assert!(!out.status.success(), "no macOS artifact must fail the job");
    }
}
