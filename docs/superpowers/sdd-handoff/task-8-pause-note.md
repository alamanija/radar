# Task 8 pause note (2026-11-05)

Task 8 (shell: sidebar, header, view routing, Tweaks, Ctrl+K) was paused partway, at the user's request, to move to another computer. The partial work is in the WIP commit that follows this file.

## Done (in the WIP commit)
- `radar-app/assets/icons/ui/*.svg`: 28 icons ported verbatim from `src/components/Icon.jsx` (24×24 viewBox, `stroke="currentColor"`, stroke-width 1.6).
- `radar-app/src/components/mod.rs` declares `icon` and `toggle`.
- `radar-app/src/components/icon.rs`:
  - `IconName` (28 variants).
  - `AppAssets`, which serves `icons/ui/*` and falls back to `gpui_kit::assets::Assets`.
  - `Icon::new(..).size().color().spin()`. When no color is set, the icon inherits the window text color.
  - Two unit tests (not yet run). One unused-import warning in a test fn.
- `radar-app/src/components/toggle.rs`: 30×18 switch with `Role::Switch` + `aria_toggled`.
- `radar-app/src/shell_tests.rs`: 16 headless tests covering the brief's 8 plus extras. They are currently RED: `cargo test -p radar-app shell` gives 0 passed, 16 failed.
- `radar-app/src/workspace.rs` is a RED stub: actions, `SourcesIntent`, Workspace fields, a no-op `init`, and an empty render. It wires the tests in via `#[path = "shell_tests.rs"] mod shell_tests`.
- `radar-app/src/main.rs` has `mod components;`.

## Remaining
- `theme.rs` additions: `DOT_NAMES` + `Tokens::dot_for`, `accent_swatch(accent, theme)`, `shadow_md(theme)`, `shadow_panel()`, `ring(width, color, inset)`.
- `components/{sidebar,header,tweaks}.rs`.
- `workspace.rs`:
  - the pattern doc comment;
  - a focus handle focused on open, with `key_context("Workspace")`;
  - `on_action` handlers;
  - `init` binding `secondary-k` via `KeyBinding::load` (`KeyBinding::new("secondary-…")` panics);
  - the layout, the FAB, and placeholder routing for the other views.
- `AppModel::run_briefing` (ruling R5) plus an `AppModel::mutate(cx, f)` helper. Remove the `#[allow(dead_code)]` attributes, and add a chrono dep for the header date.
- `main.rs`: `.with_assets(components::icon::AppAssets)` and call `workspace::init`.
- Then: GREEN, clippy, a brief launch check, commit, and write `task-8-report.md`.

## Planned deviations (state them in the report)
- The header title for Categories is "Categories" with the tag icon, because the JSX has no label for that view.
- The kbd hint is "⌘K" on macOS and "Ctrl+K" elsewhere.
- No letter-spacing, because GPUI has no such style.
- Test text assertions use `aria_label` + `find(id).label()`.

## How to resume on another machine
1. Check out the `worktree-gpui-migration` branch.
2. Restore the controller workspace: `mkdir -p .superpowers/sdd/2026-11-04-gpui-kit-migration && cp docs/superpowers/sdd-handoff/* .superpowers/sdd/2026-11-04-gpui-kit-migration/`. The `progress.md` ledger is the source of truth for what's done and every ruling.
3. Install the toolchain:
   - Rust stable (MSVC on Windows).
   - On Windows: VS 2022 Build Tools with the C++ workload, plus CMake.
   - On macOS: Xcode Command Line Tools, macOS 15+.
4. Tell Claude: "resume the GPUI migration plan with subagent-driven development". It should read the ledger and resume Task 8 from this note.
