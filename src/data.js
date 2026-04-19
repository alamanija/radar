// Radar — seed data. New users start with an empty slate: no sources, no
// categories, no articles. The onboarding tutorial on the Briefing view walks
// them through adding their first category and source.
//
// `articles` stays empty; the browser-only dev path in App.onBriefing used to
// fall back to mock rows, but that shipped dozens of hard-coded "Oatly" and
// "Pentagram" stories into people's local stores. Empty is correct — real
// data only flows in from an actual feed ingest.
export const RADAR_DATA = {
  sources: [],
  categories: [],
  articles: [],
};
