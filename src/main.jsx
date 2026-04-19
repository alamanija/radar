import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ClerkProvider } from '@clerk/clerk-react';
import App from './App.jsx';
import './styles.css';

const PUBLISHABLE_KEY = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY;

if (!PUBLISHABLE_KEY) {
  // Fail loudly in dev: Clerk can't initialise without the publishable key,
  // and a missing key would otherwise silently disable sign-in.
  throw new Error(
    'VITE_CLERK_PUBLISHABLE_KEY is not set. Copy .env.example to .env and fill it in.',
  );
}

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <ClerkProvider publishableKey={PUBLISHABLE_KEY}>
      {/* No fallback redirect URLs: sign-in runs through
          `<SignIn routing="virtual" />` in SettingsView, which flips the
          React context in-place. A redirect here would force a full page
          reload, and in Tauri's webview the dev Clerk session doesn't
          survive that reload (third-party cookie on *.clerk.accounts.dev
          is blocked and the `__clerk_db_jwt` URL param gets dropped
          across the `tauri://localhost` navigation), so the user lands
          back on a freshly-initialised, signed-out Clerk. */}
      <App />
    </ClerkProvider>
  </StrictMode>
);
