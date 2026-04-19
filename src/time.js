// Time-of-day greeting with some variety. Bucketed by local hour; the phrase
// within a bucket is keyed off the local date so it stays stable across a
// day's re-renders but rotates from day to day.
const GREETINGS = {
  lateNight: ['Still up', 'Night owl mode', 'Burning the midnight oil'],
  earlyMorn: ['Rise and shine', 'Early start', 'Up with the sun'],
  morning: ['Good morning', 'Morning', 'Top of the morning'],
  midday: ['Good afternoon', 'High noon', 'Afternoon already'],
  afternoon: ['Good afternoon', 'Afternoon', 'Back at it'],
  evening: ['Good evening', 'Evening', 'Winding down'],
  night: ['Late check-in', 'Night owl mode', 'One more scroll'],
};

function greetingBucket(hour) {
  if (hour < 5) return 'lateNight';
  if (hour < 9) return 'earlyMorn';
  if (hour < 12) return 'morning';
  if (hour < 14) return 'midday';
  if (hour < 18) return 'afternoon';
  if (hour < 22) return 'evening';
  return 'night';
}

export function greeting(name, date = new Date()) {
  const group = GREETINGS[greetingBucket(date.getHours())];
  const daySeed = date.getFullYear() * 10000 + (date.getMonth() + 1) * 100 + date.getDate();
  const phrase = group[daySeed % group.length];
  return name ? `${phrase}, ${name}.` : `${phrase}.`;
}

// "3m ago" / "2h ago" / "never", mirrors Rust's ingest::relative_time.
export function relativeTime(ms) {
  if (ms == null) return 'never';
  const diff = Math.max(0, Date.now() - Number(ms));
  const s = Math.floor(diff / 1000);
  if (s < 60) return 'just now';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 7) return `${d}d ago`;
  if (d < 30) return `${Math.floor(d / 7)}w ago`;
  return `${Math.floor(d / 30)}mo ago`;
}
