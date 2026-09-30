# Antigravity (`agy`) Accounts

Agent Control can run Antigravity with several Google accounts. Each session runs
`agy` as the account you selected. It never silently falls back to the machine's
default `agy` login.

## How it works

```
selected account ─► account ID ─► Account Manager ─► credential_ref
      ─► ~/.config/agentcontrol/credentials/agy_<id>.json (validated)
      ─► profile ~/.config/agentcontrol/profiles/<account-id>/
      ─► agy started with HOME=<profile>
      ─► agy reads <profile>/.gemini/antigravity-cli/antigravity-oauth-token
```

* **Profile isolation.** Every account has its own profile directory. The profile
  holds a private copy of that account's token. Antigravity state (token, account
  list, caches, conversations) is per-profile. The only things linked in from
  your real home are non-auth items: git/ssh/shell configuration, plus agy
  `settings.json`, `trustedFolders.json`, keybindings and the agy binary. Your
  real `~/.gemini` login is never linked into a profile.
* **Ambient credentials removed.** `GEMINI_API_KEY`, `GOOGLE_API_KEY`,
  `GOOGLE_APPLICATION_CREDENTIALS`, `GOOGLE_GENAI_USE_VERTEXAI` and
  `GOOGLE_CLOUD_ACCESS_TOKEN` are removed from the `agy` environment.
* **No account, no launch.** An `agy` session without a selected account is
  rejected. So is one whose credential is missing or unusable.
* **No fake prompt.** Interactive sessions start plain `agy`; nothing is typed
  for you.

## Add an account

```bash
agy account add                 # or: ac login / ac account add
agy account add --name "College Google"
agy account add --cli           # headless: paste the redirect address back
```

Methods:

1. **Sign in with Google in the browser.** The loopback callback, the
   authorization URL and the token exchange all use the same dynamically chosen
   `127.0.0.1` port, protected by `state` and PKCE (S256). The account is saved
   only after Google returns a real access **and** refresh token.
2. **Generate Login Link** (TUI). Nothing is opened locally. The TUI shows the
   Google sign-in link, which you can copy or send. If the link is opened on
   this computer, login finishes automatically. If it is opened on another
   device, the browser ends on an unreachable `http://127.0.0.1:<port>/oauth2callback?...`
   page. Copy that page's full address and paste it into the TUI. This is the
   handoff Google supports for installed-app clients. The link carries only
   `client_id`, `state` and the PKCE challenge. The PKCE verifier never leaves
   Agent Control, so an intercepted code is useless. `state` must match. The
   first valid code ends the login (single use). The login is discarded on
   success, on cancel (Esc) and after 5 minutes. The pasted address is masked
   in the UI and never logged.
3. **Import the existing local agy login** (only offered if
   `~/.gemini/antigravity-cli/antigravity-oauth-token` exists).

If anything fails, you see `Login failed. Account was NOT saved as authenticated.`
together with the reason, and nothing is saved. Failures include: the callback
port being unavailable, a callback error, a timeout, an invalid code, a rejected
exchange, a malformed response, or a storage error.

### OAuth client configuration

Browser sign-in needs the Antigravity OAuth client. It is **not** embedded in the
source code. Provide it via either:

* environment: `AC_AGY_OAUTH_CLIENT_ID` and `AC_AGY_OAUTH_CLIENT_SECRET`, or
* `~/.config/agentcontrol/antigravity-oauth-client.json` (mode `0600`):
  `{"client_id": "...", "client_secret": "..."}`

Antigravity uses a Google *installed application* client. For that client type
Google does not treat the secret as confidential (it ships inside the `agy`
binary). The real protection comes from the loopback redirect, `state` and PKCE.
The secret is only ever sent in the HTTPS request body. It never appears in
process arguments, logs, events, errors or the TUI. Without this configuration,
use "Import the existing local agy login".

## Select an account

```bash
agy                        # 1 usable account: used; several: arrow-key selector
agy "College Google"       # exact name (case-insensitive)
agy 01K...                 # exact account ID
agy account list           # names, IDs, status, credential validity / e-mail
```

* An unknown name prints `Account "<name>" does not exist.` and lists the
  available accounts. Nothing is started.
* If several accounts share a name, you must pick one explicitly (each is shown
  with its account ID), or start it by ID.
* Invalid numeric input is re-prompted. It never selects "account #1".
* The selector restores the terminal on Enter, Esc, `q`, Ctrl+C, Ctrl+D and on
  errors.

## Start a session with launch options (TUI)

Press `n` to open **Start Antigravity Session**. Use ↑/↓ to move between
fields and ←/→ to change a value.

* **Account**: any saved account launches directly, with no re-login. Choose
  `+ Add Antigravity Account` to add one; you return to the form afterwards.
* **Permission / Access Mode**: only modes the installed agy supports are
  listed. agy 1.2.13 supports Normal (no flag), Accept Edits
  (`--mode accept-edits`), Plan (`--mode plan`), Sandbox (`--sandbox`) and
  Dangerously Skip Permissions (`--dangerously-skip-permissions`, needs an
  explicit confirmation, applies to this session only). agy's `toolPermission`
  policies (`request-review`, `strict`, `always-proceed`, `proceed-in-sandbox`)
  can only be set in agy's shared `settings.json`, so they are not offered per
  session.
* **Working Directory**: type a path (absolute, relative or `~/…`). Tab lists
  child directories; typing filters the list. ↑/↓ or Tab selects, Enter opens
  the directory, Esc closes the list. The resolved absolute path is shown
  before you start.
* **Model**: `Default (agy setting)` or any model `agy models` reports for the
  account.

## Switch accounts

TUI: open a session → command palette → *Switch Account*. CLI:

```bash
ac session switch-account <SESSION_ID> <ACCOUNT_ID | EXACT_NAME>
```

`agy` cannot change accounts in-process, so Agent Control does a controlled
hand-off:

1. It validates the target's credential **first**. If that fails, the running
   session keeps running.
2. It snapshots and stops the old process.
3. It starts a new `agy` in the target account's own profile.

## Remove an account

```bash
agy account remove "College Google"      # or: ac account remove <ID|NAME> [--yes]
```

TUI: *Accounts* tab → select → `d` → confirm with `Enter` (`Esc` cancels).

* Removal is refused while any session uses the account:
  `Cannot remove "…". Active sessions: N. Stop the session first…`
* On removal, Agent Control deletes the database record, the credential file and
  the profile directory. Files are first moved aside atomically, then the record
  is deleted (rolled back if that fails), then the staged files are deleted.
  Symlinks inside the profile are removed without being followed, so your real
  `~/.gemini`, `~/.ssh`, `~/.gitconfig`, etc. are never touched.
* If the final deletion fails, the account is gone but the error and the leftover
  path (under `~/.config/agentcontrol/.removing/`) are shown so you can delete it.

## Credentials

* Canonical file: `~/.config/agentcontrol/credentials/agy_<id>.json` (`0600`),
  `"version": 2`, `"credential_type": "antigravity_oauth"`. It contains the
  agy-native token plus non-secret metadata (label, e-mail, source).
* Legacy files, including CLI `antigravity_<id>.json` files, are classified
  explicitly, never as "non-empty ⇒ token":
  * a usable token is migrated to v2 automatically;
  * an **empty** value, a **one-time login code** (`4/…`), or other invalid
    content is reported and never used. Remove such accounts and add them again.
* **Expiry / refresh.** A credential whose access token has expired is still
  valid if it has a refresh token; `agy` refreshes it itself. The refreshed token
  appears in the profile's token file, which is agy's own storage location.
  Agent Control writes it back to the stored credential when the session stops
  and before the next launch, but only if:
  * it is valid,
  * it is newer than the stored token, and
  * it belongs to the same Google identity.

  Agent Control never reads tokens from process output.

## Troubleshooting

| Message | Meaning / fix |
|---|---|
| `… cannot be started. Reason: No Antigravity credential is saved…` | The credential file is missing or empty. Remove the account and add it again. |
| `… Only a one-time login code was saved…` | Left over from an old, broken login. Remove the account and add it again. |
| `… The saved credential is not valid (access token expired and no refresh token…)` | Sign in again. |
| `… The refreshed Antigravity credential could not be used…` | The profile token is corrupt or belongs to another Google account. The stored credential is restored on the next launch. If it keeps happening, sign in again. |
| `Unable to start Antigravity login callback. Reason: Port unavailable` | Retry the login. |
| `Antigravity browser login is not configured` | See *OAuth client configuration*, or import the local agy login. |
| `Multiple Antigravity accounts match "…"` | Use the account ID shown (`agy <ID>`). |
