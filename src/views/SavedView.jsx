import { Card } from '../components/Card.jsx';
import { EmptyState } from './EmptyState.jsx';
import { viewStyles } from './BriefingView.jsx';

export function SavedView({ articles, onToggleRead, onToggleBookmark, showAccent, useSerif }) {
  const saved = articles.filter(a => a.bookmarked);
  return (
    <div style={viewStyles.wrap}>
      <h1 className="serif" style={viewStyles.title}>Saved</h1>
      <p style={viewStyles.subtitle}>{saved.length} articles kept for later. Pinned to the top of your briefing on request.</p>
      {saved.length === 0 ? (
        <EmptyState icon="bookmark" title="Nothing saved yet" message="Tap the bookmark icon on any card to keep it here." />
      ) : (
        <div style={viewStyles.grid}>
          {saved.map((a, i) => (
            <Card key={a.id} article={a} i={i}
                  showAccent={showAccent} useSerif={useSerif}
                  onToggleRead={() => onToggleRead(a.id)}
                  onToggleBookmark={() => onToggleBookmark(a.id)} />
          ))}
        </div>
      )}
    </div>
  );
}
