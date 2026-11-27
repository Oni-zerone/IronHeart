# IronHeart

Lightweight heartbeat service. Every registered application sends a signed ping at regular intervals: if the ping does not arrive within the expected interval, IronHeart emails the application's recipients.

- Rust backend (Axum, SQLite), Vue 3 dashboard with Vuetify.
- Google sign-in, invitation only; the master user is `ADMIN_EMAIL`.
- Pings signed with HMAC-SHA256 and an ever-increasing timestamp: the secret never travels and an intercepted request cannot be reused.
- The service always answers `204`, whatever the outcome: whoever tries to forge a ping cannot tell whether it worked. Invalid attempts are counted and summarized by email.
- Entanglement: several IronHeart instances can monitor each other.

Architecture, protocol and development rules: [CLAUDE.md](CLAUDE.md).

To integrate the heartbeat into another application: [docs/heartbeat-client.md](docs/heartbeat-client.md), a complete prompt to hand to whoever implements the client (protocol, rules, test vectors, examples in shell, Python, Node.js and Go).

## Development

```sh
mise install                       # Node and pnpm
docker compose up -d mailpit       # local SMTP, UI on http://localhost:8025
cp .env.example .env               # then fill in the variables
cd server && cargo run             # backend on :8080
cd web && pnpm install && pnpm dev # dashboard on http://localhost:5173
```

## Tests

```sh
cd server && cargo test            # unit tests, in-memory database, short load test
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

## License

[MIT](LICENSE)
