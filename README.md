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
| `MAIL_TRANSPORT` | in production | `brevo` (production) or `smtp` (default: unencrypted SMTP, meant for Mailpit in development). |
| `BREVO_API_KEY` | with `brevo` | Brevo API key. |
| `MAIL_FROM_EMAIL`, `MAIL_FROM_NAME` | with `brevo` | Sender of the alerts. On Brevo the address or its domain must be verified. |
| `DATABASE_URL` | no | `postgres://…` for an external Postgres (`?sslmode=require` for managed databases). Without it: SQLite in `/data`, which needs a volume. See [Database](#database). |
| `CLIENT_IP_HEADER` | behind a proxy | Header with the client IP: `x-forwarded-for` (last value) or `cf-connecting-ip` with Cloudflare. Used only in the invalid-attempt summaries. |
| `BIND_ADDR` | no | Listening address; `0.0.0.0:8080` in the image. |

### Keys and secrets

- **`SECRETS_KEY`** encrypts (AES-256-GCM) the secrets of the registered apps and of the entanglement links: generate it once with `openssl rand -base64 32` and keep a copy outside the server, next to the database backups. It never changes: with a different key the existing secrets cannot be decrypted, and every app would need a new key (rotated from the dashboard) and every client updated. A database backup is useless without this key.
- **App keys** are created in the dashboard: a public key ID (12 characters) and a secret `ihs_…`, shown only once. The database holds only the encrypted secret. If a secret is lost or leaked, rotate it from the app page: the old one stops working immediately.
- **`GOOGLE_CLIENT_SECRET`** belongs to the OAuth client (see below). If it leaks, add a new secret to the client on Google, deploy it, then delete the old one.
- **`BREVO_API_KEY`** is created in Brevo under *SMTP & API → API keys*. Brevo deactivates keys that go unused for 90 days, and with every app up IronHeart sends no email for months: for this reason the service checks the key at startup and then every week (`GET /v3/account`), which also keeps it in use. If Brevo refuses the key, the error is logged, the master sees `!! mail key` in the top bar, and the outgoing [entanglement](CLAUDE.md#entanglement-master-only) pings stop, so the other instances report this one as down with their own email. While refused, the key is checked again every hour and the pings resume as soon as Brevo accepts it. To fix it, create a new key, set it and restart. Give every instance its own key (better, its own Brevo account): instances that share one go down together, and none of them is left to send the alert.
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

Run on 2026-11-27 locally, in a Docker container limited to **1 vCPU and 1 GB** (the size of the production droplet), with a release build, 2,000 apps split across 20 users and the generator on the same Mac (Apple M1 Pro). One M1 CPU is probably faster than a droplet's shared vCPU: in production the numbers will be lower, and the first limit will be nginx with TLS, before the service.

| Scenario (20 s) | Requests/s | p50 | p95 | p99 | CPU | Process memory |
|---|---:|---:|---:|---:|---:|---:|
| Unknown keys, 64 connections | 34,208 | 1.8 ms | 2.9 ms | 3.8 ms | 72% | 16 MB |
| Forged signatures on real keys, 64 connections, 5,000 IPs | 27,427 | 2.0 ms | 3.7 ms | 7.7 ms | 73% | 21 MB |
| Forged signatures, 256 connections, 5,000 IPs | 32,041 | 7.5 ms | 13.6 ms | 23.0 ms | 100% | 30 MB |
| Valid pings (at most 1/s per app, so 2,000/s) | 2,099 | 1.9 ms | 6.8 ms | 11.2 ms | 29% | 35 MB |
| Attack (128 connections) + real pings from every app every 10 s, 40 s | 32,292 forged + 200 real | 3.8 ms | 6.1 ms | 9.2 ms | 98% | 43 MB (peak) |

Results:

- **Real pings survive the attack**: with more than 32,000 forged requests per second, all 2,000 apps recorded their ping and none was reported down.
- **Always `204`**: no other response across more than 3 million requests. The generator recorded 11 network errors out of 1.3 million in the mixed scenario (9 on real pings, all recovered on the next round): they do not come from the server's responses, and most likely come from Docker Desktop's port proxy.
- **Every forged attempt is counted**: 1,189,545 sent, 1,189,545 written to the summaries.
- **Email under control**: the attack on all 2,000 apps produced 20 emails, one per recipient; 2,000 apps going down together produced another 20, within one round of the monitor.
- **Peak process memory**: 43 MB. The SQLite WAL file stayed below 5 MB.
- `/health` answered in 1–3 ms throughout the tests.
- Only one round of the periodic check was slower than 5 seconds (6.8 s), with the CPU already at 100% from the attack; sending emails no longer slows it down.

### What changed since the first test

The first test, on the same machine, had shown three problems, fixed before this one:

| | Before | After |
|---|---|---|
| Emails after an attack on 2,000 apps | 2,000, one per app, sent one at a time: the deadline check stalled for minutes | 20, one per recipient, put in a separate queue |
| Valid pings at 2,000/s | CPU 100%, p99 54 ms, container memory 130 MB | CPU 29%, p99 11 ms, 35 MB |
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
