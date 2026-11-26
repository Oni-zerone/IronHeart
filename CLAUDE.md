# IronHeart

Servizio leggero di heartbeat. Gli utenti accedono con Google e registrano le proprie applicazioni, ognuna con una API key. Se un'applicazione non chiama l'endpoint di heartbeat entro l'intervallo previsto (più la tolleranza), parte una mail all'indirizzo dell'account.

## Stack

- `server/`: Rust, Axum, SQLite con sqlx (WAL), openidconnect, sessioni nel database. In seguito: `Mailer` (API HTTP in produzione, SMTP verso Mailpit in locale), dashboard incorporata con rust-embed, OpenAPI con utoipa.
- `web/` (da creare): Vite, React, TypeScript, TanStack Router e Query.
- Deploy: un solo binario in un container su una droplet DigitalOcean con CapRover, una sola istanza, volume su `/data`. L'immagine si compila in GitHub Actions e si pubblica su GHCR, non sulla droplet.

## Comandi

- `cd server && cargo run`: avvia su `127.0.0.1:8080` con il database in `server/data/ironheart.db`. Legge `.env` (modello in `.env.example`); le variabili d'ambiente hanno la precedenza.
- Senza Vite: `PUBLIC_BASE_URL=http://localhost:8080 cargo run` e apri http://localhost:8080 (pagina provvisoria con il login).
- `cd server && cargo test`: i test del database usano SQLite in memoria.
- `docker compose up -d mailpit`: SMTP locale su `:1025`, interfaccia su http://localhost:8025.

## Regole per il database (portabilità verso Postgres)

Per ora usiamo SQLite, ma il passaggio a Postgres deve restare un lavoro da uno o due giorni. Quindi:

1. **Tutte le tabelle sono `STRICT`**, così SQLite rifiuta i tipi sbagliati invece di salvarli.
2. **Tempi come `INTEGER` in millisecondi Unix UTC**, calcolati in Rust e passati come parametri. Niente `datetime('now')`, `CURRENT_TIMESTAMP` o aritmetica sulle date in SQL. La codifica di chrono in SQLite è testo con un numero variabile di decimali e non si ordina correttamente come stringa. In Postgres restano `BIGINT`, oppure diventano `timestamptz` nella migrazione dei dati.
3. **ID come UUID v7 generati in Rust**, salvati come `TEXT`. Niente `AUTOINCREMENT`.
4. **Solo SQL comune ai due database**: `INSERT … ON CONFLICT` (mai `INSERT OR REPLACE`), `RETURNING`, indici parziali. Niente `GLOB`, funzioni specifiche di SQLite o `PRAGMA` fuori da `db::connect`.
5. **Segnaposto `$1, $2…`** anche su SQLite (sqlx li accetta), così le query restano valide in Postgres.
6. **Tutte le query stanno in `server/src/db.rs`** (o in un modulo `db/` se cresce). Gli handler non scrivono SQL.
7. **Booleani come `INTEGER` 0/1** con `CHECK`, ed enumerazioni come `TEXT` con `CHECK (… IN (…))`.
8. Non scriviamo un livello di astrazione per supportare entrambi i database.

## Login e sessioni

- Google OpenID Connect con PKCE, lato server: `/auth/google/login`, `/auth/google/callback`, `POST /auth/logout`. Il redirect registrato su Google è `PUBLIC_BASE_URL` + `/auth/google/callback`.
- Il login in corso sta in `oauth_flows` (10 minuti, uso singolo) ed è legato al browser dal cookie `ih_oauth_state`.
- La sessione è il cookie `ih_session` (HttpOnly, SameSite=Lax, Secure in HTTPS, 30 giorni); nel database c'è solo lo SHA-256 del token.
- Negli handler: `CurrentUser` risponde 401 senza sessione, `Option<CurrentUser>` no.
- Si accettano solo email verificate da Google.

## Regole di dominio

- Le API key si mostrano una volta sola. Nel database finiscono solo il prefisso (per riconoscerle) e lo SHA-256.
- Il cambio di stato `up → down` è atomico (`UPDATE … WHERE state = 'up' RETURNING …`), così la mail parte una sola volta. Al ritorno dell'applicazione parte una mail di ripristino.
- Dopo un riavvio del servizio, le scadenze superate mentre il servizio era spento non generano avvisi: vengono rinviate prima di essere valutate.
