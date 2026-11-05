# Briefing sections: Top stories, New, Earlier

Date: 2026-11-05. Status: approved design, pending implementation plan.

## Goal

Make a once-a-day, top-to-bottom morning read have a clear shape and a
clear end. Today a briefing is the feeds' whole recent window, newest
first, with items already seen in earlier runs mixed in. After this
change the page leads with the few articles that matter most to the
reader, then the rest of what is new, then a "caught up" line, with
older items collapsed.

## Decisions

- **Usage model:** one briefing a day, read through. "New" means *not in
  any earlier briefing run*; everything else is "Earlier".
- **Top stories:** ranked by Claude against the reader's lens, using the
  summary request that already runs per article.
- **Earlier:** collapsed by default, with a count, below a "caught up"
  line.
- **Approach A:** "new" is derived from the archive's run history, not
  stored per article.

## 1. Data and core logic (`radar-core`)

### The "new" set

`RadarModel` gains an in-memory `new_ids: HashSet<u64>`. It is never
stored or synced; each device derives its own.

1. **Live run:** the newest archive entry whose set of article ids
   equals the set of live article ids. This matters with sync: another
   device's later run can sit at `archives[0]` while the live articles
   came from this device's own run. Read/bookmark toggles change flags,
   not ids, so the match holds.
2. **Earlier runs:** archive entries more than 6 hours
   (`SAME_BRIEFING_WINDOW_HOURS`) older than the live run. Runs closer
   than that are part of the same briefing: another synced device firing
   at the same scheduled slot, or a re-run the same morning. Older runs
   from other devices count as seen. *(Amended after the branch review:
   with "strictly older", two devices on the same synced schedule each
   saw the other's run as earlier and collapsed the whole morning under
   "Earlier".)*
3. **New:** live ids that appear in no earlier run.
4. **Fallback:** if no archive entry matches, every entry except
   `archives[0]` counts as earlier. With no live articles the set is
   empty.

Recomputed at hydration (`from_snapshot`), in `finish_briefing`, and
after an archive sync pull merges entries.

`BriefingOutcome::new_count` (the "N new articles" notification) is the
size of this set after `finish_briefing`, so the notification and the
New section always agree. The transient `prev_ids` path is replaced.

### Importance

- **Request:** `summarize.rs` adds `importance` to the structured-output
  schema as a required integer with `enum: [1, 2, 3, 4, 5]`. The system
  prompt gains one rule: how much this reader would want to read it
  today; 5 = must-read, 3 = a typical item, 1 = skip; judged against the
  Reader context when present, otherwise as a designer following the
  field.
- **Storage:** `Article.importance: Option<u8>`, `#[serde(default,
  skip_serializing_if = "Option::is_none")]`. Values outside 1–5 are
  dropped (the summary is kept). Absent on articles that were never
  summarized or were summarized before this change.
- **Cache:** `KnownSummary` gains `importance: Option<u8>`;
  `split_known` copies it onto the reused article and
  `RadarModel::known_summaries` collects it. Summaries cached before
  this change have `None` and are still reused, not re-summarized.
- **No re-scoring** when the lens or model changes (same policy as
  summaries).

### Top stories

`RadarModel::briefing_sections(kept) -> Sections { top, new, earlier }`
splits the filtered briefing articles (the existing category and
unread-only filtering of `briefing_articles`, including the `kept` set):

- `top`: up to 3 articles from the new set with `importance >= 4`,
  ordered by importance descending, then `publishedAt` descending.
- `new`: the remaining new articles, newest first.
- `earlier`: everything else, newest first.

Constants: `TOP_STORIES_MAX = 3`, `TOP_STORY_MIN_IMPORTANCE = 4`.

"Newest first" everywhere means `publishedAt` descending with undated
articles last, the order `run_briefing` already produces; sections keep
the live order rather than re-sorting.

## 2. Briefing layout (`radar-app`)

Below the subtitle and filter bar, top to bottom:

1. **Top stories**: heading "Top stories", then the `top` articles.
   Omitted when `top` is empty.
2. **New · N**: heading with the count, then the `new` articles.
   Omitted when empty.
3. **End line**: "You're caught up." after the New section; when the
   run produced no new articles at all (before filters), "Nothing new
   since your last briefing." instead.
4. **Earlier · M**: a toggle row, collapsed by default; expanded it
   shows the `earlier` articles. Omitted when `earlier` is empty.

- Each section is its own grid (`grid_box`) or list box with its heading
  above. In list mode the `VIRTUAL_MIN` threshold applies per section.
- The expanded state lives on `BriefingView` (`earlier_open`), starts
  collapsed, and is reset when the view is left (with the existing
  `reset`).
- The category chip and "Unread only" filter every section, and section
  counts follow the filters. "Mark all read" still covers the whole
  active category, including collapsed Earlier articles.
- Subtitle: "Your briefing surfaced 24 articles across 4 sources · 9 new
  · 12 unread · updated 2h ago." (the "· N new" part only when N > 0).
- Element ids for tests and accessibility: `briefing-section-top`,
  `briefing-section-new`, `briefing-caught-up`, `briefing-earlier-toggle`
  (aria label "Show earlier articles (M)" / "Hide earlier articles"),
  `briefing-section-earlier`.
- A first briefing on a fresh install: everything is new; top stories
  come from all articles. While scanning, the skeletons show as today.
- Saved and Archive views are unchanged.

## 3. Errors, compatibility, testing

- **Errors:** a failed summary leaves `importance` unset; the article
  stays in New and is scored on the next successful summary. An
  out-of-range score is dropped without failing the summary.
- **Compatibility:** `importance` is an optional field that older builds
  ignore; stored files without it are unchanged. Synced archives carry
  it opaquely. `new_ids` is never persisted.
- **Tests (first):**
  - `radar-core`: new-set derivation (normal; later run from another
    device; no matching archive; no runs); notification count equals the
    new set; `briefing_sections` (threshold, cap, tie order, category and
    unread filters, `kept`); schema and prompt contain `importance`;
    out-of-range dropped; cache carries importance.
  - `radar-app`: sections in grid and list mode; caught-up and
    nothing-new lines; Earlier collapsed by default, toggles, resets on
    leaving the view; filters apply across sections; subtitle new count.
- **Docs:** CLAUDE.md gains the sections and importance notes.

## Out of scope

- Archive storage redesign (next design).
- Re-scoring on lens change; configurable top-story count.
