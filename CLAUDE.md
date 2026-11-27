# IronHeart

Servizio leggero di heartbeat. Gli utenti accedono con Google e registrano le proprie applicazioni, ognuna con una API key. Se un'applicazione non chiama l'endpoint di heartbeat entro l'intervallo previsto (più la tolleranza), parte una mail all'indirizzo dell'account.

## Stack

- `server/`: Rust, Axum, SQLite con sqlx (WAL), openidconnect, sessioni nel database, email con Brevo (API HTTP) in produzione e SMTP verso Mailpit in locale. In seguito: dashboard incorporata con rust-embed, OpenAPI con utoipa.
- `web/`: Vue 3, TypeScript, Vite, Vue Router, Vuetify 4 con icone `@mdi/js` (SVG). Node serve solo per sviluppo e build: in produzione la dashboard è un insieme di file statici serviti dal binario Rust. PrimeVue è stato scartato perché dalla 5 non è più MIT (licenza commerciale con chiave).
- Deploy: un solo binario in un container su una droplet DigitalOcean con CapRover, una sola istanza, volume su `/data`. L'immagine si compila in GitHub Actions e si pubblica su GHCR, non sulla droplet.

## Comandi

- `cd server && cargo run`: avvia su `127.0.0.1:8080` con il database in `server/data/ironheart.db`. Legge `.env` (modello in `.env.example`); le variabili d'ambiente hanno la precedenza.
- Senza Vite: `PUBLIC_BASE_URL=http://localhost:8080 cargo run` e apri http://localhost:8080 (pagina provvisoria con il login).
- `cd server && cargo test`: i test del database usano SQLite in memoria.
- Prova manuale dell'heartbeat: lo script di esempio è nella pagina dell'app (e in `web/src/snippet.ts`). Con `MAIL_TRANSPORT=smtp cargo run` gli avvisi finiscono in Mailpit anche se `.env` dice `brevo`.
- `cd web && pnpm dev`: dashboard su http://localhost:5173; Vite inoltra `/api`, `/auth` e `/health` al backend su :8080. Con Vite, `PUBLIC_BASE_URL=http://localhost:5173`.
- `cd web && pnpm build` (controllo dei tipi + build) e `pnpm lint`.
- Node e pnpm sono fissati in `mise.toml`: `mise install`, poi `mise exec -- pnpm …` se la shell non li attiva.
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

## Utenti e ruoli

- Si entra solo su invito. `ADMIN_EMAIL` è l'utente master: entra sempre, è sempre admin (il ruolo si reimposta a ogni accesso), non si può declassare né rimuovere.
- Gli altri entrano se hanno già un account (riconosciuto dal `sub` di Google) o se c'è un invito per la loro email (`user_invites`, consumato al primo accesso in `db::login_google_user`). Senza invito il login rimanda a `/?login=denied` e non crea nulla.
- Ruoli: `admin` (invita, cambia ruolo, rimuove utenti; API `/api/admin/*`, estrattore `AdminUser`) e `user` (solo le proprie app). Le app restano personali anche per gli admin.
- Un admin non può declassare né rimuovere se stesso. Rimuovere un utente cancella a cascata app e sessioni, e le sue app vanno tolte dal registro in memoria.
- Le email (account, inviti, `ADMIN_EMAIL`) si confrontano in minuscolo.

## Entanglement (solo master)

- Questa istanza manda heartbeat ad altre istanze di IronHeart (`entanglements`), così si controllano a vicenda. Lo configura solo il master (`MasterUser`, API `/api/entanglements`, pagina `/entanglement`); è una configurazione dell'istanza, non di un utente.
- Ogni collegamento: origine dell'altra istanza (HTTPS, HTTP solo verso localhost, niente percorso), key ID e segreto di un'app creata lì, intervallo da 10 secondi a 1 giorno, attivo o in pausa. Il segreto è cifrato con `SECRETS_KEY` (dato associato `entanglement:<id>`) e non torna mai al client.
- `entanglement::spawn`: ogni 5 secondi manda i ping scaduti (`take_due_pings` sposta subito la scadenza); all'avvio tutti i collegamenti attivi mandano un ping. Timestamp sempre crescente per collegamento, anche con «Invia ora» nello stesso secondo. Redirect non seguiti, timeout 10 secondi.
- L'altra istanza risponde sempre 204: qui si registra solo se il ping è stato consegnato (`last_success_at`, `last_error`). Una chiave sbagliata la segnala l'altra istanza con i tentativi non validi.

## Email

- `MAIL_TRANSPORT=brevo` (produzione, richiede `BREVO_API_KEY`) oppure `smtp` (predefinito, `SMTP_HOST`/`SMTP_PORT`, verso Mailpit). Mittente: `MAIL_FROM_EMAIL` e `MAIL_FROM_NAME`; su Brevo il mittente o il dominio devono essere verificati.
- Una email per destinatario, così nessuno vede gli indirizzi degli altri.
- `POST /api/me/test-email` manda una mail di prova all'utente collegato.

## Heartbeat

Protocollo (riferimento per i client, test in `crypto::tests::signature_matches_reference_vector`):

- Chiave: `key_id` pubblico (12 caratteri) + segreto `ihs_…`, mostrato una sola volta. Il segreto è la chiave HMAC così com'è.
- `POST /api/v1/heartbeat` con `X-IronHeart-Key`, `X-IronHeart-Timestamp` (secondi Unix), `X-IronHeart-Signature: v1=<hex>`.
- Firma: HMAC-SHA256 di `v1\nPOST\n/api/v1/heartbeat\n<key_id>\n<timestamp>\n<sha256 hex del body>`. Body al massimo 1 KB.
- Valido se: firma corretta, timestamp entro ±5 minuti, timestamp maggiore dell'ultimo accettato (quindi al massimo un ping al secondo).

Regole che non vanno rotte:

- **La risposta è sempre 204**, per ogni esito (anche body troppo grande o chiave sconosciuta), così un attaccante non sa se ha clonato l'heartbeat. Niente 4xx, niente 429.
- **Stesso lavoro per ogni richiesta**: con un `key_id` sconosciuto si calcola comunque un HMAC con una chiave fittizia; il confronto è in tempo costante; la scrittura di un ping valido avviene in un task separato, dopo la risposta.
- **Niente database per le richieste non valide**: i segreti sono decifrati in memoria (`heartbeat::Heartbeat`), il timestamp crescente si controlla in memoria (`fetch_max`) e nel database (`record_ping` con `WHERE last_signed_ts < $ts`), i tentativi falliti si sommano in memoria e il controllo li scrive ogni 30 secondi (`failure_buckets`). Nessun log per singola richiesta.
- I segreti sono cifrati con AES-256-GCM (`SECRETS_KEY`, versione in `secret_key_version`), con l'id dell'app come dato associato.
- Quando un'app cambia (creazione, rotazione, intervallo, cancellazione) va aggiornato anche il registro in memoria (`refresh_app`, `set_timing`, `remove_app`).
- IP del client: `CLIENT_IP_HEADER` (`x-forwarded-for` → ultimo valore, scritto da nginx; `cf-connecting-ip` con Cloudflare), altrimenti l'indirizzo della connessione. Serve solo per i riepiloghi.
- Il traffico volumetrico si ferma prima del servizio (Cloudflare davanti al dominio), non nel codice.

## Controllo periodico (`monitor`, ogni 30 secondi)

1. Scrive i tentativi falliti accumulati.
2. Porta a `down` le app `up` con la scadenza passata (atomico) e avvisa; la mail cita i tentativi non validi dall'ultimo segnale.
3. Manda le mail di ripristino (`recovery_pending`, impostato dal ping che riporta l'app `up`).
4. Riepiloghi dei tentativi falliti per app: a riposo il primo parte subito, poi finestre di 5, 10, 20, 40, 80, 120 minuti; una finestra con tentativi produce un riepilogo e fa salire di livello, una senza tentativi fa scendere di un livello (mai a riposo di colpo, per non permettere raffiche di mail). Logica in `monitor::escalate`, con i test.
5. Pulisce tentativi più vecchi di 7 giorni, sessioni scadute e login abbandonati.

All'avvio le scadenze già passate ripartono da adesso (`reschedule_overdue`). Lo stato viene salvato prima di mandare la mail: un invio fallito non si ritenta.

## Regole di dominio

- Ogni app può avere un elenco opzionale di destinatari degli avvisi (`app_recipients`, email in minuscolo). Se l'elenco è vuoto, gli avvisi vanno all'email dell'account proprietario (`db::alert_recipients`). Per ora gli indirizzi aggiunti non richiedono conferma.

- App: `new` (mai un ping), `up`, `down`, `paused` (nessuna scadenza; alla ripresa si aspetta un intervallo intero). Intervallo da 30 secondi a 7 giorni, tolleranza fino a 1 giorno, al massimo 100 app per utente e 10 destinatari per app.
- Il cambio di stato `up → down` è atomico (`UPDATE … WHERE state = 'up' RETURNING …`), così la mail parte una sola volta. Al ritorno dell'applicazione parte una mail di ripristino.
- Dopo un riavvio del servizio, le scadenze superate mentre il servizio era spento non generano avvisi: vengono rinviate prima di essere valutate.
