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
      // After any sign-in / sign-up flow, Clerk redirects the browser to
      // one of these URLs. We use "/" — the Tauri webview and the Vite
      // preview both land back in the root of the app, which is where
      // the React tree boots. Without these, Clerk falls back to its
      // own hosted account page after sign-in and users end up stuck
      // on accounts.<instance>.clerk.accounts.dev.
      signInFallbackRedirectUrl="/"
      signUpFallbackRedirectUrl="/"
    >
      <App />
    </ClerkProvider>
  </StrictMode>
);
