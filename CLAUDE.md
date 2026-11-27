# IronHeart

Lightweight heartbeat service. Users sign in with Google and register their applications, each with an API key. If an application does not call the heartbeat endpoint within the expected interval (plus the grace period), an email goes to the account's address.

## Language

- **Everything in the repository is written in English**: documentation (README, `docs/`, this file), code (identifiers, comments, log messages, test names), commit messages, the dashboard UI, API error messages and alert emails.
- Do not add new text in other languages. When you touch a file that still contains Italian, translate the parts you change.
- Exception: migrations that have already been applied (`server/migrations/*/`) are never edited, not even their comments: `sqlx::migrate!` stores a checksum of each file, and a changed file stops existing databases from starting. Their Italian comments stay; new migrations are written in English.

## Stack

- `server/`: Rust, Axum, sqlx with the `Any` driver: external Postgres if `DATABASE_URL` is `postgres://…`, otherwise local SQLite (WAL, `journal_size_limit` 16 MB). openidconnect, sessions in the database, email through Brevo (HTTP API) in production and SMTP to Mailpit locally. The dashboard (`web/dist`) is embedded with rust-embed (`server/src/web.rs`: in release it is compiled in, in debug it is read from disk; without a build the temporary page is served). Later: OpenAPI with utoipa.
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
- Manual heartbeat test: the example script is on the app page (and in `web/src/snippet.ts`). With `MAIL_TRANSPORT=smtp cargo run` alerts end up in Mailpit even if `.env` says `brevo`.
- `cd web && pnpm dev`: dashboard on http://localhost:5173; Vite proxies `/api`, `/auth` and `/health` to the backend on :8080. With Vite, `PUBLIC_BASE_URL=http://localhost:5173`.
- `cd web && pnpm build` (type check + build) and `pnpm lint`.
- Node and pnpm are pinned in `mise.toml`: `mise install`, then `mise exec -- pnpm …` if the shell does not activate them.
- `docker compose up -d mailpit`: local SMTP on `:1025`, UI on http://localhost:8025.
- `docker compose up -d postgres`: local Postgres, `DATABASE_URL=postgres://ironheart:ironheart@localhost:5432/ironheart`.

## Database rules (SQLite and Postgres)

The same queries run on SQLite (local fallback, when `DATABASE_URL` is missing) and on Postgres (external, in production), through sqlx's `Any` pool (`db::Pool`). Therefore:

1. **All SQLite tables are `STRICT`**, so SQLite rejects wrong types instead of storing them.
2. **Times as `INTEGER` Unix UTC milliseconds**, computed in Rust and passed as parameters. No `datetime('now')`, `CURRENT_TIMESTAMP` or date arithmetic in SQL. chrono's SQLite encoding is text with a variable number of decimals and does not sort correctly as a string. In Postgres they are `BIGINT`.
3. **IDs as UUID v7 generated in Rust**, stored as `TEXT`. No `AUTOINCREMENT`.
4. **Only SQL common to both databases**: `INSERT … ON CONFLICT` (never `INSERT OR REPLACE`), `RETURNING`, partial indexes, `UPDATE … FROM (VALUES …)` (columns `column1…`). No `GLOB`, SQLite-specific functions or `PRAGMA` outside `db::connect`.
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
- An admin cannot demote or remove themselves. Removing a user cascades to their apps and sessions, and their apps must be removed from the in-memory registry.
- Emails (accounts, invites, `ADMIN_EMAIL`) are compared in lowercase.
- For the master only, `/api/me` includes `database` (`db::DatabaseInfo`: `postgres` or `sqlite` and where it lives, without credentials); the top bar shows it as a chip, yellow for local SQLite.

## Entanglement (master only)

- This instance sends heartbeats to other IronHeart instances (`entanglements`), so they monitor each other. Only the master configures it (`MasterUser`, API `/api/entanglements`, page `/entanglement`); it is an instance setting, not a user setting.
- Each link: origin of the other instance (HTTPS, HTTP only to localhost, no path), key ID and secret of an app created there, interval from 10 seconds to 1 day, active or paused. The secret is encrypted with `SECRETS_KEY` (associated data `entanglement:<id>`) and is never returned to the client.
- `entanglement::spawn`: every 5 seconds sends the pings that are due (`take_due_pings` moves the deadline forward immediately); at startup every active link sends a ping. The timestamp always increases per link, even with "Send now" in the same second. Redirects are not followed, 10-second timeout.
- While this instance's Brevo key is refused (`mail_check`), pings are not sent and the link records `HELD_ERROR` as its error: the other instances see this one go down and alert with their own email. At startup the first ping waits for the first key check.
- The other instance always answers 204: here we only record whether the ping was delivered (`last_success_at`, `last_error`). A wrong key is reported by the other instance as invalid attempts.

## Email

- `MAIL_TRANSPORT=brevo` (production, requires `BREVO_API_KEY`) or `smtp` (default, `SMTP_HOST`/`SMTP_PORT`, to Mailpit). Sender: `MAIL_FROM_EMAIL` and `MAIL_FROM_NAME`; on Brevo the sender or the domain must be verified.
- One email per recipient, so nobody sees the other addresses.
- `POST /api/me/test-email` sends a test email to the signed-in user.
- Brevo deactivates API keys unused for 90 days (with every app up, no email goes out). `mail_check` does a `GET /v3/account` at startup and then every 7 days (every hour after a network error, 5xx/429 or a refusal, so a refusal by mistake does not hold the pings for a week), which keeps the key in use. A 401/403 is logged as an error, shown to the master in `/api/me` (`mail_key`, the `!! mail key` chip in the top bar) and holds the entanglement pings (see Entanglement): no email is attempted, since it would go through the refused key. A network error does not hold the pings. Nothing happens with SMTP.

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
5. Cleans up attempts older than 7 days, expired sessions and abandoned logins.

At startup, deadlines that have already passed restart from now (`reschedule_overdue`). The state is saved before sending the email: a failed send is not retried.

Alert emails:

- **The monitor does not send emails**: it puts them in the queue (`outbox`), which sends them in a separate task, four at a time. App down and recovery are urgent (never dropped, they take priority); attempt summaries are dropped if the queue is full.
- **One summary per recipient**: on each round every recipient gets at most one email per type with all the apps involved (`alerts::*_digest`; with a single app it is the single-app email). Attempt summaries go out at most every 5 minutes per recipient (`FAILURE_DIGEST_MIN_MS`); meanwhile the counts are summed in memory.
- A round slower than 5 seconds is logged as a slow periodic check.

## Domain rules

- Each app can have an optional list of alert recipients (`app_recipients`, lowercase emails). If the list is empty, alerts go to the owner account's email (`db::alert_recipients`). For now added addresses do not require confirmation.

- App: `new` (never pinged), `up`, `down`, `paused` (no deadline; on resume a full interval is awaited). Interval from 30 seconds to 7 days, grace period up to 1 day, at most 100 apps per user and 10 recipients per app.
- The `up → down` transition is atomic (`UPDATE … WHERE state = 'up' RETURNING …`), so the email goes out only once. When the application comes back a recovery email is sent.
- After a service restart, deadlines that passed while the service was down do not generate alerts: they are postponed before being evaluated.
