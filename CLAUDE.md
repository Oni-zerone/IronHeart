# IronHeart

Lightweight heartbeat service. Users sign in with Google and register their applications, each with an API key. If an application does not call the heartbeat endpoint within the expected interval (plus the grace period), an alert goes to the account's address by email and/or to its linked Telegram chats.

## Language

- **Everything in the repository is written in English**: documentation (README, `docs/`, this file), code (identifiers, comments, log messages, test names), commit messages, the dashboard UI, API error messages and alert emails.
- Do not add new text in other languages. When you touch a file that still contains Italian, translate the parts you change.
- Exception: migrations that have already been applied (`server/migrations/*/`) are never edited, not even their comments: `sqlx::migrate!` stores a checksum of each file, and a changed file stops existing databases from starting. Their Italian comments stay; new migrations are written in English.

## Stack

- `server/`: Rust, Axum, sqlx with the `Any` driver: external Postgres if `DATABASE_URL` is `postgres://…`, otherwise local SQLite (WAL, `journal_size_limit` 16 MB). openidconnect, sessions in the database, email through a provider of choice (Brevo, Resend, Postmark, Mailgun, Scaleway over HTTP APIs, or standard SMTP; Mailpit locally). The dashboard (`web/dist`) is embedded with rust-embed (`server/src/web.rs`: in release it is compiled in, in debug it is read from disk; without a build the temporary page is served). Later: OpenAPI with utoipa.
- `web/`: Vue 3, TypeScript, Vite, Vue Router, no component library (Vuetify was removed in 0.2.0). Node is only for development and builds: in production the dashboard is a set of static files served by the Rust binary.

## Interface (ASCII, English)

- The interface is in English and drawn as text: black background, grey characters, light blue accents, one monospace font (JetBrains Mono, Latin subset, bundled with `@fontsource`) at a single size. Tokens and controls are in `web/src/styles.css`.
- Everything sits on a character grid: widths in `ch`, heights in whole lines of `--lh`. A block that is not a whole number of lines tall breaks the side borders of `AsciiBox`.
- Frames are `AsciiBox` (it computes its width in columns), dialogs are `AsciiDialog`/`ConfirmDialog`, app state is `StateTag` and `DeadlineBar`. Buttons are `.btn` (`[ label ]`), small actions `.link-btn` (`[copy]`), fields `.input`/`.select`, form labels `.field-label` and help `.help`.
- Pure ASCII for drawings (`+ - | ^ >`), no Unicode box-drawing characters. Vue condenses whitespace in templates: where spaces matter (inside `white-space: pre`) keep the text on one line.
- The version shown in the top bar comes from `web/package.json` (`__APP_VERSION__`); keep it equal to `server/Cargo.toml`.
- Deployment: a single binary in a container on a DigitalOcean droplet with CapRover, a single instance, volume on `/data`. To avoid losing data at every deploy an external Postgres is used (`DATABASE_URL`, with `?sslmode=require` for managed databases). The image is built in GitHub Actions and published to GHCR, not on the droplet: `Dockerfile` (web → static musl binary built on Alpine → `scratch` image with only the binary, ~10 MB; non-root user 10001, `BIND_ADDR=0.0.0.0:8080`, SQLite in `/data`). No system libraries or CA files are needed: TLS is rustls with `webpki-roots`, so never add dependencies on OpenSSL or native certificates. On musl the allocator is mimalloc (`main.rs`; musl's own halves the heartbeat throughput), with `MIMALLOC_ARENA_EAGER_COMMIT=0` and `MIMALLOC_PURGE_DELAY=0` in the image to keep memory at ~3 MiB. The image has no shell: debug with the logs, not `docker exec` and `.github/workflows/docker.yml` (tests, then amd64 and arm64 on native runners, multi-arch manifest). Tag `vX.Y.Z` → `X.Y.Z`, `X.Y`, `latest`; push on `main` → `edge`.

## Commands

- `cd server && cargo run`: starts on `127.0.0.1:8080` with the database in `server/data/ironheart.db`. Reads `.env` (template in `.env.example`); environment variables take precedence.
- Without Vite: `PUBLIC_BASE_URL=http://localhost:8080 cargo run` and open http://localhost:8080 (the dashboard if `web/dist` is built with `pnpm build`, otherwise a temporary page with the login).
- `cd server && cargo test`: database tests use in-memory SQLite (with `IRONHEART_TEST_POSTGRES_URL=postgres://…/ironheart_test` they use Postgres, a new schema per test: use a throwaway database); includes the short load test (`load_tests.rs`, `IRONHEART_LOAD_SECS` to make it longer). Load generator for real instances: `server/examples/loadtest.rs`. Query benchmark: `cargo test --release db_bench -- --ignored --nocapture` (`IRONHEART_BENCH_DATABASE_URL` for Postgres, which gets wiped). Results in README.md.
- Manual heartbeat test: the example script is on the app page (and in `web/src/snippet.ts`). With `MAIL_TRANSPORT=smtp cargo run` alerts end up in Mailpit even if `.env` names another provider.
- `cd web && pnpm dev`: dashboard on http://localhost:5173; Vite proxies `/api`, `/auth` and `/health` to the backend on :8080. With Vite, `PUBLIC_BASE_URL=http://localhost:5173`.
- `cd web && pnpm build` (type check + build) and `pnpm lint`.
- Node and pnpm are pinned in `mise.toml`: `mise install`, then `mise exec -- pnpm …` if the shell does not activate them.
- `docker compose up -d mailpit`: local SMTP on `:1025`, UI on http://localhost:8025.
- `docker compose up -d postgres`: local Postgres, `DATABASE_URL=postgres://ironheart:ironheart@localhost:5432/ironheart`.

## CI and PR review

- `.github/workflows/ci.yml` on every PR: `cargo clippy --all-targets -- -D warnings`, `cargo test` without the load test (`--skip load_tests::`), then in `web/` oxlint, eslint (without `--fix`) and `pnpm build`. Run the same commands locally before opening a PR.
- `.github/workflows/gemini-review.yml` + `.github/scripts/gemini-review.mjs`: one Gemini call per push with the diff and this file as the project rules; comments only, incremental after the first review, it re-evaluates its open conversations and resolves those it accepts. Needs the `GEMINI_API_KEY` secret (skipped without it); optional variables `GEMINI_MODEL` and `REVIEW_RUNS_ON`. Full re-review: `gh workflow run gemini-review.yml -f pr=<n> -f full=true`.
- `.github/workflows/conversations.yml`: the `Conversations` commit status is red while a review thread is unresolved. Resolving a thread starts no workflow: refresh with `gh workflow run conversations.yml -f pr=<n>`.

## Database rules (SQLite and Postgres)

The same queries run on SQLite (local fallback, when `DATABASE_URL` is missing) and on Postgres (external, in production), through sqlx's `Any` pool (`db::Pool`). Therefore:

1. **All SQLite tables are `STRICT`**, so SQLite rejects wrong types instead of storing them.
2. **Times as `INTEGER` Unix UTC milliseconds**, computed in Rust and passed as parameters. No `datetime('now')`, `CURRENT_TIMESTAMP` or date arithmetic in SQL. chrono's SQLite encoding is text with a variable number of decimals and does not sort correctly as a string. In Postgres they are `BIGINT`.
3. **IDs as UUID v7 generated in Rust**, stored as `TEXT`. No `AUTOINCREMENT`.
4. **Only SQL common to both databases**: `INSERT … ON CONFLICT` (never `INSERT OR REPLACE`), `RETURNING`, partial indexes, `UPDATE … FROM (VALUES …)` (columns `column1…`). No `GLOB`, SQLite-specific functions or `PRAGMA` outside `db::connect`. Only exception: a SQLite migration that rebuilds a table referenced by others turns `foreign_keys` off and on around the rebuild, as `-- no-transaction` with its own `BEGIN`/`COMMIT` (0009); if it fails the service does not start.
5. **`$1, $2…` placeholders** on SQLite too (sqlx accepts them), so queries stay valid in Postgres.
6. **All queries live in `server/src/db.rs`** (or in a `db/` module if it grows). Handlers do not write SQL.
7. **Booleans as `INTEGER` 0/1** with `CHECK`, and enumerations as `TEXT` with `CHECK (… IN (…))`.
8. **Migrations in two folders**, `migrations/sqlite` and `migrations/postgres`: every schema change goes into both, with the same number. In Postgres all integers are `BIGINT` (booleans and numeric enumerations too) and blobs are `BYTEA`.
9. **Types the `Any` driver reads the same way**: only `i64`, `String`, `Vec<u8>` and their `Option`. No `bool`, neither when reading nor as a parameter (`EXISTS` → `COUNT(*)`, booleans as `i64`, see `Flag`); `SUM` always with `CAST(… AS BIGINT)` (in Postgres it would be `NUMERIC`).
10. **Few statements per round**: with a remote Postgres every statement is a network round trip. Bulk writes (`record_pings`, `insert_failure_buckets`) use a single statement per 500 rows.
11. Tests run on SQLite; before changing SQL, also run them with `IRONHEART_TEST_POSTGRES_URL`. We do not write abstraction layers beyond the `Any` pool.

## Login and sessions

- Google OpenID Connect with PKCE, server side: `/auth/google/login`, `/auth/google/callback`, `POST /auth/logout`. The redirect registered on Google is `PUBLIC_BASE_URL` + `/auth/google/callback`.
- The login in progress is stored in `oauth_flows` (10 minutes, single use) and is bound to the browser by the `ih_oauth_state` cookie.
- The session is the `ih_session` cookie (HttpOnly, SameSite=Lax, Secure over HTTPS, 30 days); the database only holds the SHA-256 of the token.
- In handlers: `CurrentUser` answers 401 without a session, `Option<CurrentUser>` does not.
- Only emails verified by Google are accepted.

## Users and roles

- Access is by invitation only. `ADMIN_EMAIL` is the master user: always allowed in, always admin (the role is reset at every sign-in), cannot be demoted or removed.
- Others get in if they already have an account (recognized by Google's `sub`) or if there is an invite for their email (`user_invites`, consumed at first sign-in in `db::login_google_user`). Without an invite the login redirects to `/?login=denied` and creates nothing.
- Roles: `admin` (invites, changes roles, removes users; API `/api/admin/*`, `AdminUser` extractor) and `user` (only their own apps). Apps stay personal for admins too.
- An admin cannot demote or remove themselves. Removing a user cascades to their personal apps and sessions (their group apps stay with the groups), and those apps must be removed from the in-memory registry.
- Emails (accounts, invites, `ADMIN_EMAIL`) are compared in lowercase.
- For the master only, `/api/me` includes `database` (`db::DatabaseInfo`: `postgres` or `sqlite` and where it lives, without credentials); the status row shows it as a chip, amber for local SQLite.
- Top of every page: one row with `/apps /groups /telegram`, the email and sign out; under the ECG line the status row, with the alert channels (`AlertChannels part="row"`) on the left and, for admins, `/users` (and for the master `/entanglement`, the database and the email key chips) aligned right.
- Errors and alerts use the amber `--alert` token (`.err`): `!!` notices, stopped apps, invalid attempts, destructive actions, the SQLite chip. `--hi` is only emphasis.

## Groups

- Users gather in groups (`groups`, `group_members`; module `groups`, queries in `db/groups.rs`). Any user can be a member, the master included, and a user can be in several groups. At most 50 groups per instance, 100 members per group, names up to 80 characters, unique ignoring case (`name_key`, lowercase computed in Rust).
- Roles: instance admins create and delete groups (`/api/admin/groups`), appoint the first group admin and can manage any group. A group admin (`group_members.role = 'admin'`) invites, removes, promotes and demotes members and renames the group. A group always keeps at least one group admin (`MemberChange::LastAdmin`); when the last admin's account is removed, the oldest remaining member is promoted (`promote_successors` in `db::delete_user`).
- Joining is always an invitation the person accepts (`group_invites`, 7 days, only for existing accounts, shown on `/apps` under pending). `POST /api/groups/{id}/invites` answers `202 {"result":"sent"}` for every address (privacy): unknown addresses and existing members are dropped silently, and the email goes through the outbox so the request time does not tell either. Group admins never see pending invitations. At most 50 invitations per group admin per day, counted in memory on every request (`groups::DailyLimit`).
- Apps belong to a user or to a group (`apps.user_id` / `apps.group_id`, exactly one: CHECK in migration 0009, which rebuilds the table on SQLite with `-- no-transaction` and the foreign keys off, so `app_recipients` and `failure_buckets` survive). Every query on a user's apps goes through one rule in `db/apps.rs`: `select_apps!` / `visible_to!` (their own apps and their groups' apps) and `managed_by!` (their own and those of the groups where they are group admin). Anything else answers 404.
- Rights on group apps: every member sees, edits, pauses and rotates the key, and creates apps in the group; only group admins delete them (403 for a plain member) or move them out. `AppView` carries `owner`, `group_role` and `can_manage`, and the dashboard hides what is not allowed.
- Limits: 100 personal apps per user and 100 apps per group, counted separately (`count_owned_apps`). A group that owns apps cannot be deleted (422, and `ON DELETE RESTRICT`).
- Alerts of a group app without explicit recipients go to every member (`db::alert_recipients`); Telegram follows the emails as usual. With email off, a group app needs a linked chat of at least one member.
- Exception: an instance admin who is also admin of the group turns an unknown address into an instance invite (`user_invites`, role `user`) with the group attached (`user_invite_groups`); the membership is added at the first sign-in in `db::login_google_user`, with no accept step, and the invite email mentions the group. The answer is then `instance_invite`. The instance invite rules do not change otherwise.

## Moving apps (`transfers`, queries in `db/transfers.rs`)

- An app keeps its id, key, state, deadline, recipients and history when it changes owner; the in-memory heartbeat registry does not know the owner and is not touched.
- Immediate moves (`POST /api/apps/{id}/move`, `{ "to": { "kind": "user" } }` or `{ "kind": "group", "id" }`): a user moves their personal app into one of their groups (any member); a group admin moves a group app to themselves or into another group they belong to. A plain member cannot move an app out of a group (403). Atomic `UPDATE` with a check on the current owner (`MoveOutcome::Stale`) and the target limit in the same transaction.
- To another person it is a request (`app_transfers`, migration 0010): `POST /api/apps/{id}/transfer {email}` by the owner or an admin of the owning group answers `202` with the same body whether or not an account has the address (the row is stored either way). The recipient is told through the outbox by email, or with email off in their linked Telegram chats; nothing is sent to an unknown address. At most one pending request per app (a new one cancels the old), 20 requests per user per day, 7 days to answer. `pending_transfer` is in `AppView` only for those who manage the app.
- The recipient sees the request on `/apps` (pending box) and accepts (`/api/transfers/{id}/accept`: the app becomes personal, if its owner did not change meanwhile and they have room) or rejects (the sender is told). A move, a deletion or a new request cancels the pending one.
- Removing a user: `DELETE /api/admin/users/{id}?move_apps_to=<group>` moves their personal apps into that group instead of deleting them; their pending requests (sent or received) are cancelled.
- The periodic check marks expired requests and deletes closed ones after 30 days.

## Entanglement (master only)

- This instance sends heartbeats to other IronHeart instances (`entanglements`), so they monitor each other. Only the master configures it (`MasterUser`, API `/api/entanglements`, page `/entanglement`); it is an instance setting, not a user setting.
- Each link: origin of the other instance (HTTPS, HTTP only to localhost, no path), key ID and secret of an app created there, interval from 10 seconds to 1 day, active or paused. The secret is encrypted with `SECRETS_KEY` (associated data `entanglement:<id>`) and is never returned to the client.
- `entanglement::spawn`: every 5 seconds sends the pings that are due (`take_due_pings` moves the deadline forward immediately); at startup every active link sends a ping. The timestamp always increases per link, even with "Send now" in the same second. Redirects are not followed, 10-second timeout.
- While no alert channel reaches the master (`channels::held`, see "Alert channels"), pings are not sent and the link records `HELD_ERROR` as its error: the other instances see this one go down and alert with their own channels. At startup the first ping waits for the first email and Telegram checks.
- The other instance always answers 204: here we only record whether the ping was delivered (`last_success_at`, `last_error`). A wrong key is reported by the other instance as invalid attempts.

## Telegram

- One bot per instance (`TELEGRAM_BOT_TOKEN`), module `telegram/`. The token is checked with `getMe` like an email key (`Telegram::health`, a `mail_check::KeyMonitor`): at startup (with backoff until the first answer), then daily, hourly while it fails. When it starts working: `deleteWebhook`, `setMyCommands` (`bot::COMMANDS`); long polling with `getUpdates` runs only while the bot is active. A 401/404, also during polling, marks the token refused; polling network errors start an outage.
- **Visibility**: token missing or empty → Telegram does not exist (`alerts.telegram` absent, no nav link, no page). Token set → `/telegram` is in the nav; while the bot is not active (refused, or never answered) the page shows only the error and `/api/me/telegram*` answers 404. `/api/me` includes `telegram: { bot_username }` only while the bot is active.
- The token is in the Bot API URL: log request errors with `without_url()`, never the URL.
- Linking: `POST /api/me/telegram/link` creates a token (single use, 10 minutes, SHA-256 in `telegram_link_tokens`) and the deep link `t.me/<bot>?start=<token>`; `/start <token>` consumes it and stores the chat in `telegram_links`. A chat belongs to one account (`chat_id` unique), an account can have several chats. Only private chats.
- Alerts follow the email recipients: the monitor maps each recipient email to the linked chats of the account with that email (`db::telegram_routes`, one query per round, only when there is something to send) and queues a Telegram message next to the email (`outbox::Message`). 403 or "chat not found" disables the link (`disabled_at`); the next message from the chat enables it again.
- Commands (`telegram::bot`) are scoped to the account's `user_id` in the queries; a chat that is not linked learns nothing. `/pause` and `/resume` ask for confirmation with an inline button and go through `apps::set_paused` (database and in-memory registry). At most 20 commands per chat per minute.
- Texts are plain text (no parse mode), in English, built in `telegram::text` (alerts) and `telegram::bot` (commands).

## Alert channels

- Module `channels`. Mode from the configuration: `mail+telegram`, `mail` (no Telegram token), `telegram` (`MAIL_TRANSPORT=none`; without a Telegram token the startup fails). With email off nothing is queued by email, the invite email is skipped, `/api/me/test-email` answers 404, and creating an app requires a linked Telegram chat.
- Health per channel (`mail_check::Health`): `checking`, `ok`, `unreachable` (network error, less than an hour, tracked by `failing_since`), `unverified` (SMTP without login), `broken` (key refused, or unreachable for over an hour). An alert email that fails asks for an early key check (`KeyMonitor::recheck_soon`).
- **Hold rule** (`Reach::held`, tested): the entanglement pings stop when no channel reaches the master and at least one is broken. Email reaches the master if it is not broken and not unverified; Telegram only with at least one active chat of the master. SMTP without login never holds by itself.
- When a channel becomes broken or works again, the master is told once on the other channels that are not broken (SMTP without login included), through the outbox. State in memory, checked every minute: after a restart with a broken channel the notice goes out again.
- `/api/me` → `alerts` (`channels::Status`) for every user: mode, health of each configured channel, whether the user has a linked chat, and for the master whether the pings are held. The main page shows it (`AlertChannels.vue`).

## Email

- `MAIL_TRANSPORT` picks the provider: `brevo`, `resend`, `postmark`, `mailgun`, `scaleway` (HTTP APIs through `reqwest`), `smtp` (default, `lettre`) or `none` (email off, see "Alert channels"). No SDKs. Sender: `MAIL_FROM_EMAIL` and `MAIL_FROM_NAME`, to be verified with the provider. The variables of each provider are in `.env.example`.
- Code in `server/src/mail/`: `mod.rs` (`Mailer`, `Email`, the `Transport` enum, `check` for HTTP responses: 2xx or an error with status and truncated body), one file per provider. No trait, a `match`. Configuration in `config::MailConfig::from_vars` (pure function, with tests): a missing variable or an unknown transport stops the startup with the variable's name. The log names only the provider, never the credentials.
- `smtp` is the fallback for every other provider: `SMTP_HOST`, `SMTP_PORT`, `SMTP_USERNAME`/`SMTP_PASSWORD` (both or neither), `SMTP_TLS` = `none` | `starttls` | `tls` (rustls, like `reqwest`; no OpenSSL). Without `SMTP_TLS`: `tls` with `SMTP_PORT=465`, `none` towards `localhost`/`127.0.0.1`/`::1`/`mailpit`, otherwise `starttls`; default port 1025, 587 or 465. Credentials without TLS towards a non-local host: the startup fails. Values are trimmed, except `SMTP_PASSWORD`; environment variables that are not UTF-8 are skipped. Mailpit works with no configuration.
- The HTTP APIs also work where outgoing SMTP is blocked (some DigitalOcean accounts). One HTTP client with a 15-second timeout; in tests the base URL points to an axum server on `127.0.0.1`.
- One email per recipient, so nobody sees the other addresses. No retries and no failover between providers.
- `POST /api/me/test-email` sends a test email to the signed-in user.
- Only the transport named in `MAIL_TRANSPORT` is on: the other providers' variables are ignored, and only its key is checked.
- `mail_check` checks the key at startup and then every day (every hour after a network error, 5xx/429, an unexpected answer or a refusal, so a refusal by mistake does not hold the pings for long). A refusal is logged as an error, shown to the master in `/api/me` (`mail_key`, with `provider`; the `!! mail key` chip in the top bar) and holds the entanglement pings (see Entanglement): no email is attempted, since it would go through the refused key. A network error does not hold the pings. The check, per provider (`Mailer::check_key`):
  - Brevo: `GET /v3/account`, 2xx valid, 401/403 refused. Brevo deactivates keys unused for 90 days (with every app up no email goes out), and this read keeps the key in use.
  - Postmark: `GET /server` with the server token, 2xx valid, 401 refused.
  - Resend, Mailgun, Scaleway: a send with an empty body (Scaleway: only `project_id`), so nothing goes out. 400/422 means the key was accepted and the body refused (valid); 401/403 refused, and so is 404 (wrong domain, region or project: no email can go out); anything else is an unexpected answer. A read cannot be used: keys that may only send (Resend "Sending access", Mailgun domain sending keys, Scaleway `TransactionalEmailEmailApiCreate`) are refused on reads.
  - SMTP with credentials: connects and logs in (`test_connection`); a permanent (5xx) answer, no common login mechanism or a TLS failure is a refusal; a timeout or network error is not. SMTP without credentials (Mailpit): no check.
  - Per the providers' documentation, only Brevo expires keys for inactivity. Resend, Postmark, Mailgun and Scaleway keys stay valid until deleted, but Mailgun (`expiration`) and Scaleway (expiration date, or an organization maximum) keys can be created with a fixed lifetime: the daily check notices within a day.
- Every email (alerts, invite, test) is drawn with `mail_layout::Letter` in the dashboard's ASCII style: a 56-column grid with the top bar, the ECG line (flat when an app is down), an `AsciiBox`-style frame and a `[ button ]`. The HTML part is one `<pre>` with inline styles on black; the text part is the same drawing with the URL below the frame. Text is wrapped in Rust, so body copy stays ASCII (`-`, `"`, `...`) to keep the borders aligned.

## Heartbeat

Protocol (reference for clients, test in `crypto::tests::signature_matches_reference_vector`). The guide for client integrators is `docs/heartbeat-client.md`: if the protocol changes, the guide, test vectors and tests must be updated together.

- Key: public `key_id` (12 characters) + `ihs_…` secret, shown only once. The secret is the HMAC key as is.
- `POST /api/v1/heartbeat` with `X-IronHeart-Key`, `X-IronHeart-Timestamp` (Unix seconds), `X-IronHeart-Signature: v1=<hex>`.
- Signature: HMAC-SHA256 of `v1\nPOST\n/api/v1/heartbeat\n<key_id>\n<timestamp>\n<sha256 hex of the body>`. Body at most 1 KB.
- Valid if: correct signature, timestamp within ±5 minutes, timestamp greater than the last accepted one (so at most one ping per second).

Rules that must not be broken:

- **The response is always 204**, for every outcome (even an oversized body or an unknown key), so an attacker cannot tell whether they cloned the heartbeat. No 4xx, no 429.
- **Same work for every request**: with an unknown `key_id` an HMAC is still computed with a dummy key; the comparison is constant time; a valid ping stays in memory and does not touch the database during the request.
- **No database during the request**: secrets are decrypted in memory (`heartbeat::Heartbeat`), the increasing timestamp is checked in memory (`fetch_max`); valid pings (the latest per app) are written in bulk once per second (`spawn_writer`, `db::record_pings`, an `UPDATE … FROM (VALUES …)` with `last_signed_ts < $ts`) and at shutdown; failed attempts are summed in memory and the monitor writes them every 30 seconds (`failure_buckets`). No logging per request.
- Secrets are encrypted with AES-256-GCM (`SECRETS_KEY`, version in `secret_key_version`), with the app id as associated data.
- When an app changes (creation, rotation, interval, deletion) the in-memory registry must be updated too (`refresh_app`, `set_timing`, `remove_app`).
- Client IP: `CLIENT_IP_HEADER` (`x-forwarded-for` → last value, written by nginx; `cf-connecting-ip` with Cloudflare), otherwise the connection address. Used only for the summaries.
- Volumetric traffic is stopped before the service (Cloudflare in front of the domain), not in the code.

## Periodic check (`monitor`, every 30 seconds)

1. Writes the accumulated failed attempts.
2. Moves `up` apps past their deadline to `down` (atomically) and alerts; the email mentions the invalid attempts since the last signal.
3. Sends the recovery emails (`recovery_pending`, set by the ping that brings the app back `up`).
4. Failed-attempt summaries per app: at rest the first one goes out immediately, then windows of 5, 10, 20, 40, 80, 120 minutes; a window with attempts produces a summary and moves up a level, one without attempts moves down a level (never straight back to rest, to prevent bursts of emails). Logic in `monitor::escalate`, with tests.
5. Cleans up attempts older than 7 days, expired sessions, abandoned logins, expired Telegram link tokens and expired group invitations; expires transfer requests and deletes closed ones after 30 days.

At startup, deadlines that have already passed restart from now (`reschedule_overdue`). The state is saved before sending the email: a failed send is not retried.

Alert emails:

- **The monitor does not send emails**: it puts them in the queue (`outbox`), which sends them in a separate task, four at a time. App down and recovery are urgent (never dropped, they take priority); attempt summaries are dropped if the queue is full.
- **One summary per recipient**: on each round every recipient gets at most one email per type with all the apps involved (`alerts::*_digest`; with a single app it is the single-app email). Attempt summaries go out at most every 5 minutes per recipient (`FAILURE_DIGEST_MIN_MS`); meanwhile the counts are summed in memory.
- A round slower than 5 seconds is logged as a slow periodic check.

## Domain rules

- Each app can have an optional list of alert recipients (`app_recipients`, lowercase emails). If the list is empty, alerts go to the owner account's email, or to every member of the owning group (`db::alert_recipients`). For now added addresses do not require confirmation.

- App: `new` (never pinged), `up`, `down`, `paused` (no deadline; on resume a full interval is awaited). Interval from 30 seconds to 7 days, grace period up to 1 day, at most 100 personal apps per user, 100 apps per group and 10 recipients per app.
- The `up → down` transition is atomic (`UPDATE … WHERE state = 'up' RETURNING …`), so the email goes out only once. When the application comes back a recovery email is sent.
- After a service restart, deadlines that passed while the service was down do not generate alerts: they are postponed before being evaluated.
