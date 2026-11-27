<div align="center">
<pre>
                  .-=====-.       .-=====-.                 
                :*#########*-   -*#########*:               
               =##########  #+.+#############=              
              :##########    #*###############:             
              :#########  #  #################:             
                         ###  ###############+              
 . - --- ----==.=###########   #############=.==---- --- - .
                 .=##########  ##                           
                   .=########      #####=.                  
                     .-*######    ###*-.                    
                        .=####   ##=.                       
                          .=#####=.                         
                            .+#+.                           
                              -                             
</pre>
<pre>
 ___                 _   _                 _   
|_ _|_ __ ___  _ __ | | | | ___  __ _ _ __| |_ 
 | || '__/ _ \| '_ \| |_| |/ _ \/ _` | '__| __|
 | || | | (_) | | | |  _  |  __/ (_| | |  | |_ 
|___|_|  \___/|_| |_|_| |_|\___|\__,_|_|   \__|
</pre>
</div>

# IronHeart

Lightweight heartbeat service. Every registered application sends a signed ping at regular intervals: if the ping does not arrive within the expected interval, IronHeart emails the application's recipients.

- Rust backend (Axum; external Postgres or local SQLite), Vue 3 dashboard with an ASCII interface (no component library).
- Google sign-in, invitation only; the master user is `ADMIN_EMAIL`.
- Pings signed with HMAC-SHA256 and an ever-increasing timestamp: the secret never travels and an intercepted request cannot be reused.
- The service always answers `204`, whatever the outcome: whoever tries to forge a ping cannot tell whether it worked. Invalid attempts are counted and summarized by email.
- Entanglement: several IronHeart instances can monitor each other.

Architecture, protocol and development rules: [CLAUDE.md](CLAUDE.md).

To integrate the heartbeat into another application: [docs/heartbeat-client.md](docs/heartbeat-client.md), a complete prompt to hand to whoever implements the client (protocol, rules, test vectors, examples in shell, Python, Node.js and Go).

## Installation

The image is published on GHCR for `linux/amd64` and `linux/arm64`. It contains only a static binary with the dashboard embedded: ~10 MB on disk (~5 MB to download), ~3 MiB of RAM at rest, ~5 MiB at 34,000 heartbeats/s. There is no shell inside: use the logs (`docker logs`).

```sh
docker run -d --name ironheart -p 8080:8080 \
  -v ironheart-data:/data \
  -e PUBLIC_BASE_URL=https://ironheart.example.com \
  -e GOOGLE_CLIENT_ID=… -e GOOGLE_CLIENT_SECRET=… \
  -e SECRETS_KEY="$(openssl rand -base64 32)" \
  -e ADMIN_EMAIL=you@example.com \
  -e MAIL_TRANSPORT=brevo -e BREVO_API_KEY=… -e MAIL_FROM_EMAIL=noreply@example.com \
  ghcr.io/oni-zerone/ironheart:latest
```

Any container platform works: the service needs one port (`8080`), the environment variables below and, only with SQLite, a volume on `/data`. Run a single instance: deadlines, the key registry and the alert queue live in the process. Put it behind a reverse proxy that terminates HTTPS. Health check: `GET /health`.

Tags: `latest` and `X.Y.Z` / `X.Y` for releases, `edge` for the latest commit on `main`.

### Configuration

All variables are listed in [.env.example](.env.example).

| Variable | Required | Meaning |
|---|---|---|
| `PUBLIC_BASE_URL` | yes | Address users open in the browser, e.g. `https://ironheart.example.com`, without a trailing `/`. It builds the Google redirect URI and the links in the emails; with `https://` the cookies are `Secure`. |
| `ADMIN_EMAIL` | yes | Master user: can always sign in, is always admin, invites the others. |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | yes | OAuth client, see [Google sign-in](#google-sign-in). |
| `SECRETS_KEY` | yes | Master key for the app secrets, see [Keys and secrets](#keys-and-secrets). |
| `MAIL_TRANSPORT` | in production | `brevo`, `resend`, `postmark`, `mailgun`, `scaleway` or `smtp` (default: Mailpit in development). Each needs its own variables, see [Email](#email). `none` turns email off: alerts only on [Telegram](#telegram), which then becomes required. |
| `MAIL_FROM_EMAIL`, `MAIL_FROM_NAME` | with a provider | Sender of the alerts. The address or its domain must be verified with the provider. |
| `DATABASE_URL` | no | `postgres://…` for an external Postgres (`?sslmode=require` for managed databases). Without it: SQLite in `/data`, which needs a volume. See [Database](#database). |
| `CLIENT_IP_HEADER` | behind a proxy | Header with the client IP: `x-forwarded-for` (last value) or `cf-connecting-ip` with Cloudflare. Used only in the invalid-attempt summaries. |
| `TELEGRAM_BOT_TOKEN` | no | Token of the instance's Telegram bot (BotFather). Without it, or if Telegram refuses it, Telegram does not appear anywhere. See [Telegram](#telegram). |
| `BIND_ADDR` | no | Listening address; `0.0.0.0:8080` in the image. |

### Keys and secrets

- **`SECRETS_KEY`** encrypts (AES-256-GCM) the secrets of the registered apps and of the entanglement links: generate it once with `openssl rand -base64 32` and keep a copy outside the server, next to the database backups. It never changes: with a different key the existing secrets cannot be decrypted, and every app would need a new key (rotated from the dashboard) and every client updated. A database backup is useless without this key.
- **App keys** are created in the dashboard: a public key ID (12 characters) and a secret `ihs_…`, shown only once. The database holds only the encrypted secret. If a secret is lost or leaked, rotate it from the app page: the old one stops working immediately.
- **`GOOGLE_CLIENT_SECRET`** belongs to the OAuth client (see below). If it leaks, add a new secret to the client on Google, deploy it, then delete the old one.
- **The email key** (`BREVO_API_KEY`, `RESEND_API_KEY`, …) is checked at startup and then every day, see [Email](#email). Give every instance its own key (better, its own account with the provider): instances that share one go down together, and none of them is left to send the alert.
- Configuration goes in the platform's environment variables or secret store, never in the image or in the repository (`.env` is ignored by git).

### Google sign-in

IronHeart signs users in with Google OpenID Connect (server side, with PKCE) and only asks for `openid`, `email` and `profile`. In the [Google Cloud Console](https://console.cloud.google.com/):

1. Create a project (or pick an existing one) and open **Google Auth Platform**.
2. **Branding**: app name, support email and developer contact. Under *Authorized domains* add the domain of `PUBLIC_BASE_URL` (e.g. `example.com`).
3. **Audience**: choose *External* to allow any Google account (access is still invite-only inside IronHeart), or *Internal* to limit it to your Google Workspace organization. An External app in *Testing* only accepts the test users listed on this page: add yourself, or click *Publish app*. With only these basic scopes no Google verification is required.
4. **Data access**: add the scopes `openid`, `.../auth/userinfo.email` and `.../auth/userinfo.profile`.
5. **Clients → Create client**, type *Web application*. Under *Authorized redirect URIs* add `PUBLIC_BASE_URL` + `/auth/google/callback`, e.g. `https://ironheart.example.com/auth/google/callback`. For development also add `http://localhost:8080/auth/google/callback` and, with Vite, `http://localhost:5173/auth/google/callback` (or use a separate client). *Authorized JavaScript origins* are not needed.
6. Copy the client ID and the client secret into `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`. Copy the secret right away: the console may not show it again.

The redirect URI must match `PUBLIC_BASE_URL` exactly (scheme, host, port): a mismatch shows up as `redirect_uri_mismatch` on Google's page. Only addresses verified by Google are accepted. On a new instance only `ADMIN_EMAIL` can sign in: everyone else needs an invite from an admin (page `/users`), otherwise the login ends on `/?login=denied` and no account is created.

### Releasing

The `Docker` workflow (`.github/workflows/docker.yml`) runs the server tests, builds the image on native amd64 and arm64 runners and publishes it on GHCR. A release is a tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Every push on `main` updates `edge`. The image can also be built locally with `docker build -t ironheart .`.

## Development

```sh
mise install                       # Node and pnpm
docker compose up -d mailpit       # local SMTP, UI on http://localhost:8025
cp .env.example .env               # then fill in the variables
cd server && cargo run             # backend on :8080
cd web && pnpm install && pnpm dev # dashboard on http://localhost:5173
```

## Email

The provider is chosen with `MAIL_TRANSPORT`; the sender is `MAIL_FROM_EMAIL` / `MAIL_FROM_NAME` and must be verified with the provider (address or domain). Every provider is one HTTP call or standard SMTP: no SDKs. The variables of each one are in [.env.example](.env.example).

| `MAIL_TRANSPORT` | Type | Variables |
|---|---|---|
| `brevo` | HTTP API | `BREVO_API_KEY` |
| `resend` | HTTP API | `RESEND_API_KEY` |
| `postmark` | HTTP API | `POSTMARK_SERVER_TOKEN`, `POSTMARK_MESSAGE_STREAM` (default `outbound`) |
| `mailgun` | HTTP API | `MAILGUN_API_KEY`, `MAILGUN_DOMAIN`, `MAILGUN_REGION` = `us` \| `eu` (default `us`) |
| `scaleway` | HTTP API (Transactional Email) | `SCW_SECRET_KEY`, `SCW_PROJECT_ID`, `SCW_REGION` (default `fr-par`) |
| `smtp` (default) | SMTP | `SMTP_HOST`, `SMTP_PORT`, `SMTP_USERNAME`, `SMTP_PASSWORD`, `SMTP_TLS` = `none` \| `starttls` \| `tls` |

- `smtp` is the fallback for any other provider (Brevo SMTP, Amazon SES via SMTP, Gmail, Fastmail, your own server). Without `SMTP_TLS` it is `tls` with `SMTP_PORT=465`, `none` towards `localhost`/`127.0.0.1`/`mailpit` and `starttls` elsewhere; the default port follows the TLS mode (1025, 587, 465). With credentials and no TLS towards a non-local host the service refuses to start. Locally it reaches Mailpit with no configuration.
- The HTTP APIs also work where outgoing SMTP ports are blocked (some DigitalOcean accounts).
- A wrong configuration stops the startup with the name of the missing variable; the log names only the provider, never the credentials. `POST /api/me/test-email` sends a test email to the signed-in user.
- A failed send is not retried and there is no failover between providers.
- Only the provider named in `MAIL_TRANSPORT` is on: the variables of the others are ignored.

### Key check

With every app up IronHeart sends no email for months, so a broken key would only show at the first alert. The service therefore checks the key of the configured provider at startup and then every day:

| Provider | Check | Expiry, per the provider's documentation |
|---|---|---|
| Brevo | `GET /v3/account` | Keys unused for 90 days are deactivated: the daily read keeps the key in use. |
| Resend | a send with an empty body, refused for the missing fields: nothing goes out | None: keys stay valid until deleted (unused ones are only flagged after 30 days). |
| Postmark | `GET /server` | None documented. |
| Mailgun | a send with no fields, refused for the missing fields | None for inactivity; a key can be created with a fixed lifetime (`expiration`). |
| Scaleway | a send with only the project, refused for the missing fields | None for inactivity; a key can have an expiration date, or the organization a maximum duration. |
| SMTP | connection and login (only with `SMTP_USERNAME`) | Depends on the server. |

Sends are used instead of reads because keys that may only send (Resend "Sending access", Mailgun domain sending keys, a Scaleway key with only `TransactionalEmailEmailApiCreate`) are refused on reads.

If the provider refuses the key (401/403; 404 to the empty send, i.e. a wrong Mailgun domain or region; a 5xx, missing login mechanism or TLS failure on SMTP), the error is logged, the master sees `!! mail key` in the top bar, and the master is told on Telegram when it works, and the outgoing [entanglement](CLAUDE.md#entanglement-master-only) pings stop if no channel reaches the master any more (see [Alert channels](#alert-channels)). While refused, the key is checked again every hour. To fix it, create a new key, set it and restart. A network error or an outage at the provider counts as a failure only after an hour; an alert email that fails to send triggers an early check.

## Telegram

Each instance can have its own Telegram bot: alerts also arrive in the linked chats, and the bot answers a few fixed commands about the user's applications. It is optional.

1. Create the bot with [@BotFather](https://t.me/BotFather) (`/newbot`) and set its token in `TELEGRAM_BOT_TOKEN`.
2. Restart. At startup the service calls `getMe`: if Telegram accepts the token, the log says `telegram: bot @<name> active`, the commands are registered with `setMyCommands`, and every user sees `/telegram` in the dashboard. Without a token (missing or empty) Telegram does not appear anywhere. With a token Telegram refuses, `/telegram` only shows the error and the API answers 404.
3. Each user opens `/telegram`, presses "link telegram" and opens the link (valid 10 minutes, single use) on the device with Telegram. "test telegram" on the main page sends a test message.

- One bot per instance, read with long polling (`getUpdates`): no public endpoint is needed, but the same token must not be used by two instances (Telegram gives the updates to only one reader).
- Telegram follows the email recipients: a chat receives an app's alerts when the address of its account receives them.
- Commands: `/apps`, `/down`, `/status`, `/attempts`, `/pause`, `/resume` (with a confirmation button), `/test`, `/stop`, `/help`. They answer only in private chats linked to an account, and only about that account's applications.
- If a user blocks the bot, the chat is disabled until it writes to the bot again.
- The token is checked like an email key: daily, hourly while it fails. Polling errors count as an outage.
- `TELEGRAM_API_URL` (default `https://api.telegram.org`) points the bot to a local [Bot API server](https://github.com/tdlib/telegram-bot-api), or to a mock in the load test.

### Alert channels

An instance sends alerts by email, on Telegram, or both, depending on the configuration; the main page shows the channels and their state to every user.

| Mode | Configuration |
|---|---|
| mail + telegram | an email provider and `TELEGRAM_BOT_TOKEN` |
| mail | an email provider, no Telegram token |
| telegram | `MAIL_TRANSPORT=none` and `TELEGRAM_BOT_TOKEN` (without a token the service does not start) |

- A channel is **broken** when its key is refused, or when it has been unreachable for more than an hour.
- When a channel breaks, the master is told once on the other channel (an email if Telegram broke, a Telegram message if email broke), and again when it works again. The entanglement pings go on.
- When no channel reaches the master any more and one of them is broken, the entanglement pings stop: the linked instances report this one as down and alert in its place. Telegram reaches the master only through a chat linked to the master's account: with email broken and no master chat, the pings stop. Without entanglement nobody outside is told: configure at least one linked instance.
- SMTP without login cannot be checked: it never stops the pings by itself, but when Telegram breaks the notice is attempted by email and the pings stop anyway.
- Telegram only: a user must link a chat before creating applications, and the invites are not sent by email (tell the invitee the instance's address).

## Database

The database is chosen with `DATABASE_URL`:

- `postgres://user:password@host:5432/ironheart?sslmode=require`: external Postgres. This is the choice for production, because the data survives container deploys. Tables are created automatically at first startup (`server/migrations/postgres`).
- not set: local SQLite file in `data/ironheart.db` (`server/migrations/sqlite`). Handy in development; in a container it is lost at every deploy, unless `data/` is on a volume.

At startup the log says which database is in use (without credentials). To try Postgres locally: `docker compose up -d postgres` and `DATABASE_URL=postgres://ironheart:ironheart@localhost:5432/ironheart`.

## Tests

```sh
cd server && cargo test            # unit tests, in-memory database, short load test
IRONHEART_TEST_POSTGRES_URL=postgres://ironheart:ironheart@localhost:5432/ironheart_test \
    cargo test                     # same suite on Postgres (one schema per test: throwaway database)
cd web && pnpm build && pnpm lint  # dashboard type check, build and lint
```

The suite includes a load test (`server/src/load_tests.rs`): the server runs on a local port with 200 apps; 32 workers send forged signatures from many IPs while every app sends one valid ping per second. The test checks that every response is `204`, that no real ping is lost, that every forged attempt is counted and that invalid requests do not write to the database. It lasts 2 seconds; for a longer run:

```sh
IRONHEART_LOAD_SECS=30 cargo test --release load_tests -- --nocapture
```

To measure a real instance there is the load generator `server/examples/loadtest.rs` (modes `unknown`, `bad`, `valid`, `mixed`):

```sh
cargo run --release --example loadtest -- --url https://heartbeat.example.com \
    --mode mixed --keys keys.csv --concurrency 128 --spoof-ips 5000 --duration 40
```

## Load test

Run on 2026-11-27 locally, with the release Docker image (`scratch`, the same one used in production) limited to **1 vCPU and 1 GB** (the size of the production droplet), local SQLite on a volume, 2,000 apps split across 20 users and the generator on the same Mac (Apple M1 Pro). One M1 CPU is probably faster than a droplet's shared vCPU: in production the numbers will be lower, and the first limit will be nginx with TLS, before the service.

| Scenario (20 s) | Requests/s | p50 | p95 | p99 | CPU | Container memory (peak) |
|---|---:|---:|---:|---:|---:|---:|
| Unknown keys, 64 connections | 37,108 | 1.6 ms | 2.7 ms | 3.4 ms | 70% | 9 MiB |
| Forged signatures on real keys, 64 connections, 5,000 IPs | 34,481 | 1.7 ms | 2.8 ms | 3.9 ms | 82% | 19 MiB |
| Forged signatures, 256 connections, 5,000 IPs | 40,279 | 6.4 ms | 9.5 ms | 11.0 ms | 85% | 22 MiB |
| Valid pings (at most 1/s per app, so 2,000/s) | 2,099 | 1.8 ms | 6.4 ms | 9.7 ms | 15% | 23 MiB |
| Attack (128 connections) + real pings from every app every 10 s, 40 s | 37,944 forged + 200 real | 3.3 ms | 5.1 ms | 6.1 ms | 84% | 25 MiB |

Results:

- **Real pings survive the attack**: with almost 38,000 forged requests per second, all 2,000 apps recorded their ping and none was reported down. The state was read from the volume after stopping the container.
- **Always `204`**: no other response and no network error across more than 3 million requests.
- **Every forged attempt is counted**: 3,013,390 sent, 3,013,390 written to the summaries.
- **Email under control**: the attack on all 2,000 apps produced 20 emails, one per recipient.
- **No slow round** of the periodic check and no errors in the log. The SQLite database ended at 7 MB.
- The Telegram channel (PR #9, with a fake Bot API and one linked chat per user) gave the same numbers, within 3%, and 20 messages, one per chat.

**Bug found by this test**: the `scratch` image has no `/tmp`, so SQLite could not create the temporary file it needs for the bulk ping write (`UPDATE … FROM (VALUES …)` with 500 rows) and failed with `disk I/O error` (`SQLITE_IOERR_GETTEMPPATH`): valid pings got a `204` but were never written, and all 2,000 apps stayed `new`. The earlier test had been run before the switch to `scratch`. Now every connection sets `PRAGMA temp_store = MEMORY`: only that scratch data of a single statement stays in memory, while the database and its WAL stay on the volume as before.

### What changed since the first test

The first test, on the same machine, had shown three problems, fixed before this one:

| | Before | After |
|---|---|---|
| Emails after an attack on 2,000 apps | 2,000, one per app, sent one at a time: the deadline check stalled for minutes | 20, one per recipient, put in a separate queue |
| Valid pings at 2,000/s | CPU 100%, p99 54 ms, container memory 130 MB | CPU 15%, p99 10 ms, 23 MiB |
| SQLite WAL file | 163 MB, never shrinking | below 5 MB (`journal_size_limit` at 16 MB) |

The fixes:

1. **Email queue**: the periodic check puts alerts in a queue and moves on; a separate task sends them, four at a time. App-down and recovery alerts take priority and are never dropped; attempt summaries are dropped if the queue is full.
2. **One summary per recipient**: on each round every recipient gets at most one email per type, listing all the apps involved. Invalid-attempt summaries arrive at most every 5 minutes per recipient, even when the windows of their apps close at different times; meanwhile the counts add up.
3. **`journal_size_limit`**: after every checkpoint the WAL file goes back to at most 16 MB.
4. **Valid pings written in bulk**: a valid ping stays in memory (the latest per app) and once per second they are all written in one transaction; at shutdown the remaining ones are written.

## SQLite and Postgres

Measured on 2026-11-27 on the same Mac (Apple M1 Pro, release build), with Postgres 17 in Docker on the same machine: this is the most favorable case for Postgres, because a network round trip costs about 0.5 ms. A managed database in another datacenter will cost more for every statement.

**HTTP load test** (`load_tests`, 20 s, 200 apps, 32 workers with forged signatures): no difference. The heartbeat endpoint does not touch the database, and with all three databases the service handles about 69,000 forged requests per second, recording all the real pings (200/200). In 1 run out of 10 the generator had client-side connection errors (29 requests out of 1.4 million, with the machine under load); the next 6 runs, alternating databases, all passed.

**Database operations** (`db_bench`, 2,000 apps across 20 users, median of 3 runs):

| Operation | SQLite, before | SQLite, now (`Any` driver) | Postgres |
|---|---:|---:|---:|
| Bulk ping write (2,000 pings, every second) | 19 ms | 28 ms | 44 ms |
| Failed attempts of 2,000 apps (one monitor round) | 172 ms | 190 ms | 80 ms |
| Monitor round at rest | 0.5 ms | 0.5 ms | 6.5 ms |
| 2,000 apps going down together (`mark_overdue_down`) | 5 ms | 6 ms | 31 ms |
| Key registry at startup (2,000 apps) | 3.8 ms | 4.4 ms | 4.3 ms |
| Session (every dashboard request) | 0.05 ms | 0.05 ms | 0.85 ms |
| List of 100 apps with attempt totals | 0.5 ms | 0.5 ms | 2.6 ms |
| Alert recipients (per app) | 0.05 ms | 0.05 ms | 0.9 ms |

- **The `Any` driver costs little**: with the same queries, SQLite stayed within 10–15% of before.
- **On Postgres the cost is the number of statements**: the first version wrote pings with one `UPDATE` per app and took **1,007 ms** for 2,000 pings, i.e. the writer's whole second (and more with a remote database). Now it is a single `UPDATE … FROM (VALUES …)` per 500 pings: 44 ms. Same for failed attempts (from 1,694 to 80 ms). On SQLite the bulk version is a bit slower (28 ms versus 19), but it stays at 3% of the second.
- Dashboard requests cost about 1 ms per query instead of 0.05: not noticeable.
- **To keep an eye on**: when many apps change state together, the monitor still runs a few queries per app (recipients, attempt summaries). With 2,000 apps going down together, reading the recipients alone takes about 2 seconds on a local Postgres; with a remote database at 3 ms it could exceed 5 seconds and show up in the log as a slow periodic check. If that happens, they should be batched like the pings.

## License

[MIT](LICENSE)
