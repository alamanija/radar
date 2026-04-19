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
    <ClerkProvider
      publishableKey={PUBLISHABLE_KEY}
      // Clerk's default after-sign-in redirect calls `window.location.assign`,
      // which hard-reloads the app. Under `tauri://localhost` the reload drops
      // the `__clerk_db_jwt` (third-party cookie on *.clerk.accounts.dev is
      // blocked, URL param strips across the navigation), so Clerk boots a
      // fresh anonymous client and the just-created session becomes
      // unreachable. Overriding router* with `history.pushState`/`replaceState`
      // keeps the redirect in-page; `isSignedIn` flipping re-renders the UI.
      routerPush={(to) => window.history.pushState(null, '', to)}
      routerReplace={(to) => window.history.replaceState(null, '', to)}
    >
      <App />
    </ClerkProvider>
  </StrictMode>
);
