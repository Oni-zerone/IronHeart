# IronHeart

Servizio leggero di heartbeat. Ogni applicazione registrata manda un ping firmato a intervalli regolari: se il ping non arriva entro l'intervallo previsto, IronHeart manda una mail ai destinatari dell'applicazione.

- Backend in Rust (Axum, SQLite), dashboard in Vue 3 con Vuetify.
- Accesso con Google, solo su invito; l'utente master è `ADMIN_EMAIL`.
- Ping firmati con HMAC-SHA256 e timestamp sempre crescente: il segreto non viaggia mai e una richiesta intercettata non si può riusare.
- Il servizio risponde sempre `204`, qualunque sia l'esito: chi prova a falsificare un ping non sa se ci è riuscito. I tentativi non validi vengono contati e riassunti via mail.
- Entanglement: più istanze di IronHeart possono controllarsi a vicenda.

Dettagli di architettura, protocollo e regole di sviluppo: [CLAUDE.md](CLAUDE.md).

Per integrare l'heartbeat in un'altra applicazione: [docs/heartbeat-client.md](docs/heartbeat-client.md), un prompt completo da dare a chi implementa il client (protocollo, regole, vettori di prova, esempi in shell, Python, Node.js e Go).

## Sviluppo

```sh
mise install                       # Node e pnpm
docker compose up -d mailpit       # SMTP locale, interfaccia su http://localhost:8025
cp .env.example .env               # poi compila le variabili
cd server && cargo run             # backend su :8080
cd web && pnpm install && pnpm dev # dashboard su http://localhost:5173
```

## Test

```sh
cd server && cargo test            # unità, database in memoria, prova di carico breve
cd web && pnpm build && pnpm lint  # controllo dei tipi, build e lint della dashboard
```

La suite comprende una prova di carico (`server/src/load_tests.rs`): il server gira su una porta locale con 200 app; 32 worker mandano firme false da molti IP mentre ogni app manda un ping valido al secondo. Il test verifica che ogni risposta sia `204`, che nessun ping vero vada perso, che ogni tentativo falso sia contato e che le richieste non valide non scrivano nel database. Dura 2 secondi; per una prova più lunga:

```sh
IRONHEART_LOAD_SECS=30 cargo test --release load_tests -- --nocapture
```

Per misurare un'istanza vera c'è il generatore di carico `server/examples/loadtest.rs` (modalità `unknown`, `bad`, `valid`, `mixed`):

```sh
cargo run --release --example loadtest -- --url https://heartbeat.esempio.it \
    --mode mixed --keys keys.csv --concurrency 128 --spoof-ips 5000 --duration 40
```

## Prova di carico

Eseguita il 27/11/2026 in locale, su un container Docker limitato a **1 vCPU e 1 GB** (la taglia del droplet di produzione), con build release, 2.000 app divise su 20 utenti e il generatore sullo stesso Mac (Apple M1 Pro). Una CPU dell'M1 è probabilmente più veloce della vCPU condivisa di un droplet: in produzione i numeri saranno più bassi, e il primo limite sarà nginx con TLS, prima del servizio.

| Scenario (20 s) | Richieste/s | p50 | p95 | p99 | CPU | Memoria del processo |
|---|---:|---:|---:|---:|---:|---:|
| Chiavi sconosciute, 64 connessioni | 34.208 | 1,8 ms | 2,9 ms | 3,8 ms | 72% | 16 MB |
| Firme false su chiavi vere, 64 connessioni, 5.000 IP | 27.427 | 2,0 ms | 3,7 ms | 7,7 ms | 73% | 21 MB |
| Firme false, 256 connessioni, 5.000 IP | 32.041 | 7,5 ms | 13,6 ms | 23,0 ms | 100% | 30 MB |
| Ping validi (al massimo 1/s per app, quindi 2.000/s) | 2.099 | 1,9 ms | 6,8 ms | 11,2 ms | 29% | 35 MB |
| Attacco (128 connessioni) + ping veri di tutte le app ogni 10 s, 40 s | 32.292 falsi + 200 veri | 3,8 ms | 6,1 ms | 9,2 ms | 98% | 43 MB (picco) |

Risultati:

- **I ping veri sopravvivono all'attacco**: con oltre 32.000 richieste false al secondo, tutte le 2.000 app hanno registrato il ping e nessuna è risultata ferma.
- **Sempre `204`**: nessuna risposta diversa su oltre 3 milioni di richieste. Il generatore ha registrato 11 errori di rete su 1,3 milioni nello scenario misto (9 sui ping veri, tutti recuperati al giro successivo): non dipendono dalle risposte del server, molto probabilmente vengono dal proxy delle porte di Docker Desktop.
- **Ogni tentativo falso è contato**: 1.189.545 mandati, 1.189.545 scritti nei riepiloghi.
- **Mail sotto controllo**: l'attacco su tutte le 2.000 app ha prodotto 20 mail, una per destinatario; 2.000 app ferme insieme ne hanno prodotte altre 20, entro un giro del controllo.
- **Memoria massima del processo**: 43 MB. Il file WAL di SQLite è rimasto sotto i 5 MB.
- `/health` ha risposto in 1–3 ms durante tutte le prove.
- Un solo giro del controllo periodico è stato più lento di 5 secondi (6,8 s), con la CPU già al 100% per l'attacco; gli invii delle mail non lo rallentano più.

### Cosa è cambiato rispetto alla prima prova

La prima prova, sulla stessa macchina, aveva mostrato tre problemi, corretti prima di questa:

| | Prima | Dopo |
|---|---|---|
| Mail dopo un attacco su 2.000 app | 2.000, una per app, mandate una alla volta: il controllo delle scadenze restava fermo per minuti | 20, una per destinatario, messe in una coda separata |
| Ping validi a 2.000/s | CPU 100%, p99 54 ms, memoria del container 130 MB | CPU 29%, p99 11 ms, 35 MB |
| File WAL di SQLite | 163 MB, senza mai ridursi | sotto i 5 MB (`journal_size_limit` a 16 MB) |

Le correzioni:

1. **Coda delle mail**: il controllo periodico mette gli avvisi in coda e prosegue; un task separato li manda, quattro alla volta. Gli avvisi di app ferma o tornata attiva hanno la precedenza e non si scartano; i riepiloghi dei tentativi si scartano se la coda è piena.
2. **Un riepilogo per destinatario**: a ogni giro ogni destinatario riceve al massimo una mail per tipo, che elenca tutte le app coinvolte. I riepiloghi dei tentativi non validi arrivano al massimo ogni 5 minuti per destinatario, anche quando le finestre delle sue app si chiudono in momenti diversi; i conteggi nel frattempo si sommano.
3. **`journal_size_limit`**: dopo ogni checkpoint il file WAL torna al massimo a 16 MB.
4. **Ping validi scritti in blocco**: il ping valido resta in memoria (l'ultimo per app) e una volta al secondo si scrivono tutti in una transazione; allo spegnimento si scrivono quelli rimasti.

## Licenza

[MIT](LICENSE)
