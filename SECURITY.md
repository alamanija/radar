# Security Policy

Thanks for taking the time to look. Radar takes a small but non-trivial
amount of user data across the wire and the disk, so vulnerability reports
are genuinely appreciated.

## Supported Versions

Only the **latest released version** on [GitHub Releases](https://github.com/alamanija/radar/releases/latest)
receives security fixes. If you're running an older build, update first —
the in-app auto-updater makes this one click in Settings → Updates. This
is a solo-maintained project; backporting to older versions isn't
realistic.

| Version | Supported |
| ------- | --------- |
| Latest release (`main`) | ✅ |
| Anything older | ❌ |

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security reports.** Use one
of the private channels below instead.

### Preferred: GitHub Private Security Advisories

The most convenient path for both of us:

1. Go to https://github.com/alamanija/radar/security/advisories/new
2. Fill in a description, affected component, and reproduction steps
3. Submit — only you and the maintainer see it

GitHub's private-advisory workflow gives us a collaboration space, CVE
assignment (if warranted), and a clean public disclosure once a fix
ships.

### Alternative: Email

If you'd rather not use GitHub: **security@getradar.xyz**

Please include enough detail to reproduce. If you want to encrypt, ask
for a PGP key in an initial message and I'll respond with one.

## Response Expectations

- **Acknowledgement:** within 5 business days.
- **Triage + severity assessment:** within 10 business days.
- **Fix + release:** timeline depends on severity — critical issues
  (auth bypass, remote code execution, credential exfiltration) are
  prioritized over everything else; moderate issues land in the next
  scheduled release.
- **Public disclosure:** coordinated with the reporter. Default is 90
  days after a fix ships, earlier if there's active exploitation.

This is a solo project; timelines slip occasionally. You'll get an
update if that happens rather than silence.

## What's In Scope

Any of the following in the **latest release**:

- **Client (Tauri desktop app)**
  - Anthropic API key handling (keyring storage, memory scrubbing,
    accidental logging)
  - Clerk JWT handling (token leakage, misuse, CSRF in the auth flow)
  - Local storage / `radar.json` contents (any path that leaks data
    across user accounts or exposes secrets to unintended code)
  - Updater signature verification (anything that lets an attacker
    deliver an unsigned or incorrectly-signed update)
  - Tauri IPC permissions and command handlers in `src-tauri/src/`
  - RSS feed parsing / URL handling (SSRF, malformed-feed crashes)
- **Server (`radar-server`)**
  - Authentication bypass, privilege escalation between Clerk users
  - Data leakage across `clerk_user_id` boundaries in any `/sync/*`
    endpoint
  - Injection vulnerabilities (SQL, command, header)
  - `If-Match` / etag semantics being spoofable to overwrite data
  - Any endpoint returning another user's rows
- **Build / supply chain**
  - Release workflow (`.github/workflows/release.yml`) — anything
    that lets a third party publish a signed release, tamper with
    artifacts, or exfiltrate the Tauri signing key
  - `render.yaml` / `Dockerfile` — secrets leakage, privilege issues

## What's Out of Scope

- **Issues that require physical access to an unlocked device**, such
  as reading the Tauri store or the user's keychain. The OS keychain
  is considered trustworthy — if an attacker has already compromised
  the user's login session, Radar isn't the right threat model.
- **Social engineering** of users or maintainers.
- **Denial of service via raw HTTP flooding** of the public server.
  Rate limiting is per-user, not per-IP; if you can take down the
  free-tier host with a handful of requests you probably can too for
  a random npm package's download endpoint. Not interesting unless
  there's an amplification factor.
- **Missing security hardening that isn't a vulnerability**, e.g. "the
  server doesn't set HSTS." Report these as regular issues/PRs.
- **Third-party service weaknesses** (Clerk, Anthropic, Render, Neon,
  GitHub). Report those to the upstream vendor.
- **Reports generated purely by automated scanners** without a proof
  of concept or impact analysis.

## Safe Harbor

If you make a good-faith effort to comply with this policy — stick to
in-scope issues, don't exfiltrate data you don't own, and give me a
reasonable chance to fix it before going public — I won't pursue legal
action. Please don't do anything that would constitute a crime outside
the context of this policy (e.g. pivoting from a bug in Radar into
Clerk's or Render's infrastructure).

## Credit

Confirmed reports are credited in the public GitHub Security Advisory
(unless you request otherwise) and in the release notes for the fix.
No monetary bounty — this is a hobby project.
