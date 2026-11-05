### Task 8: Shell: sidebar, header, view routing, Tweaks panel, Ctrl+K action

**Files:**
- Create: `radar-app/src/components/{sidebar,header,icon,toggle,tweaks}.rs`
- Modify: `radar-app/src/workspace.rs`
- Reference: `src/components/{Sidebar,Header,Icon,Toggle,Tweaks}.jsx`, `src/styles.css`

**Interfaces:**
- Consumes: `AppModel`, `theme::t`.
- Produces:
  - `actions!(radar, [OpenSearch, RunBriefing, ToggleSidebar, ToggleTheme])`. Bindings: `secondary-k` → `OpenSearch`, context `"Workspace"`. The workspace focuses its handle on window open.
  - `icon::Icon::new(IconName)` covers the icon set in `Icon.jsx`, drawn as embedded SVG assets under `assets/icons/ui/` and copied from the JSX paths.
  - `Workspace { search_open: bool, tweaks_open: bool, sources_intent: Option<SourcesIntent> }`.

- [ ] **Step 1: Write failing headless tests** (`#[gpui_kit::test]`):
  - `sidebar_click_sets_view`: clicking "Saved" sets `prefs.view == Saved` and persists it.
  - `sidebar_category_filter_sets_active_category`.
  - `sidebar_counts`: the unread count badge equals `unread_count()`.
  - `header_theme_toggle_flips_and_reapplies_tokens`.
  - `ctrl_k_opens_search`: dispatch `OpenSearch` and `search_open == true`.
  - `toggle_sidebar_collapses` (`prefs.sidebar_collapsed`).
  - `tweaks_accent_swatch_updates_prefs_and_tokens`.
  - `sidebar_footer_shows_profile_name_or_account`.
- [ ] **Step 2: Run `cargo test -p radar-app shell`.** Expected: FAIL.
- [ ] **Step 3: Implement it.**
  - Match the JSX layout: 260 px sidebar, collapsible to 0. Animation is optional.
  - Header: sidebar toggle, title per view, the grid/list segmented control shown only on the briefing view, a search bar that opens the palette, theme toggle, and a "Run briefing" button with a spinner while `scanning`.
  - Tweaks: a FAB at bottom-right 20 px with Theme, View, Density, Accent, Serif, Accent bars, and Sidebar collapsed.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Visual check.** Run the app side by side with `radar-handoff/project/` open in a browser, in light and dark. Fix any spacing or color drift over 2 px.
- [ ] **Step 6: Commit** with `git commit -m "feat(app): sidebar, header, tweaks, keybindings"`.

