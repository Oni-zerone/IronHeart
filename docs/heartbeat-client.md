# Prompt: integrare l'heartbeat di IronHeart in un'applicazione

Copia questo documento così com'è a chi deve implementare il client (una persona o un assistente AI), insieme al codice dell'applicazione da integrare. Contiene tutto quello che serve: non c'è bisogno di leggere il codice di IronHeart.

---

## Compito

Aggiungi a questa applicazione un **heartbeat verso IronHeart**: una richiesta HTTP firmata, mandata a intervalli regolari, che dimostra che l'applicazione è viva. Se IronHeart non riceve un heartbeat valido entro l'intervallo configurato per l'applicazione, manda un avviso via mail.

L'heartbeat deve partire **dal punto che dimostra davvero che l'applicazione funziona**: per un job pianificato, alla fine di un'esecuzione riuscita; per un servizio, da un task periodico del processo principale (non da un processo separato che resterebbe vivo anche se il servizio è bloccato).

## Configurazione

Tre valori, letti dalla configurazione dell'applicazione (variabili d'ambiente o secret manager), **mai scritti nel codice**:

| Variabile | Esempio | Note |
|---|---|---|
| `IRONHEART_URL` | `https://heartbeat.esempio.it` | Solo l'origine, senza `/` finale e senza percorso. |
| `IRONHEART_KEY_ID` | `q88gwdjtcfdr` | Pubblico: identifica l'applicazione. |
| `IRONHEART_SECRET` | `ihs_…` | Segreto. Non va mai loggato, mostrato o mandato in rete. |

Key ID e segreto si ottengono dalla dashboard di IronHeart quando si crea l'applicazione (il segreto si vede una sola volta). Se uno dei tre valori manca, l'heartbeat va disattivato con un avviso nel log all'avvio, senza bloccare l'applicazione.

## Protocollo

### Richiesta

```
POST {IRONHEART_URL}/api/v1/heartbeat
X-IronHeart-Key:       {IRONHEART_KEY_ID}
X-IronHeart-Timestamp: {timestamp}
X-IronHeart-Signature: v1={firma}
```

- `timestamp`: secondi Unix (UTC) come intero decimale, per esempio `1764000000`.
- Body: **vuoto** (consigliato). Se ne mandi uno, al massimo 1 KB, e va firmato esattamente con i byte inviati.
- `Content-Type` non è richiesto.

### Firma

1. Calcola lo SHA-256 del body in esadecimale minuscolo. Con il body vuoto è sempre:
   `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
2. Costruisci la stringa canonica, con `\n` (LF, un solo carattere) come separatore e **senza** `\n` finale:
   ```
   v1\nPOST\n/api/v1/heartbeat\n{key_id}\n{timestamp}\n{sha256_body_hex}
   ```
3. Calcola HMAC-SHA256 della stringa canonica (codificata in UTF-8) usando come chiave **la stringa del segreto così com'è**, in UTF-8, compreso il prefisso `ihs_` (non va decodificata da base64).
4. L'intestazione è `v1=` seguito dall'HMAC in esadecimale minuscolo (64 caratteri).

### Vettori di prova

Usali in un test automatico del client: se non tornano, la firma è sbagliata.

| | Valore |
|---|---|
| key ID | `abcdefghijkm` |
| segreto | `ihs_test` |
| timestamp | `1764000000` |
| firma, body vuoto | `395ebdbcd9e58a46090ac3c6ca626a0e00b94301076532b944841b3338cd5f01` |
| body | `{"status":"ok"}` (15 byte, senza spazi né a capo) |
| SHA-256 del body | `a29ee2b15c494311c52521766e44af56a3ad2248e7a8ab465e5206463c13d288` |
| firma, con quel body | `f9cefe637f8e3e4b9688b79f948bf544702d7b76e983f927d6293749d20fe9b3` |

### Risposta

**IronHeart risponde sempre `204 No Content`**, anche se la firma è sbagliata, la chiave non esiste o il timestamp è fuori tolleranza: è una scelta di sicurezza, per non far capire a chi prova a falsificare un ping se ci è riuscito. Quindi:

- un `204` significa solo che la richiesta è arrivata, **non** che sia valida;
- qualunque altra risposta (`5xx`, redirect, errori di rete, timeout) significa che la richiesta non è arrivata al servizio;
- la validità si verifica solo nella dashboard di IronHeart: «ultimo segnale» deve aggiornarsi e la sezione «Tentativi non validi» deve restare vuota.

## Regole che il server applica

Una richiesta è valida solo se:

1. la firma corrisponde;
2. il timestamp è entro **±5 minuti** dall'ora del server;
3. il timestamp è **strettamente maggiore** di quello dell'ultimo heartbeat accettato per la stessa chiave.

Conseguenze per il client:

- **Al massimo un heartbeat al secondo** per chiave: due richieste nello stesso secondo, la seconda viene scartata (e contata come «richiesta ripetuta»).
- **Mai riusare una richiesta**: a ogni tentativo, compresi i retry, ricalcola timestamp e firma. Il timestamp deve sempre crescere: se il nuovo valore non è maggiore dell'ultimo mandato, usa l'ultimo + 1.
- **L'orologio della macchina deve essere sincronizzato** (NTP). Con più di 5 minuti di scarto ogni heartbeat viene scartato come «orologio fuori tolleranza».
- Una chiave appartiene a **una sola istanza** dell'applicazione. Se ne girano più copie (più repliche, più server), ognuna deve avere la propria applicazione e la propria chiave su IronHeart: copie che condividono la chiave si scartano a vicenda i ping.

## Comportamento richiesto al client

- **Intervallo**: manda l'heartbeat con un intervallo **più breve** di quello configurato su IronHeart, per esempio la metà, così un ping perso non basta a far scattare l'avviso. Rendilo configurabile (`IRONHEART_INTERVAL_SECS`, se serve).
- **Timeout**: 10 secondi per richiesta.
- **Retry**: se la richiesta non arriva (errore di rete, timeout, risposta diversa da `204`), riprova al massimo 2–3 volte con attesa crescente (per esempio 2 s, 5 s, 10 s), ricalcolando timestamp e firma a ogni tentativo; poi rinuncia fino al ciclo successivo. Non riprovare dopo un `204`.
- **Redirect**: non seguirli. L'URL firmato è fisso; un redirect indica una configurazione sbagliata.
- **HTTPS** obbligatorio in produzione (HTTP solo verso `localhost` in sviluppo).
- **Non bloccare l'applicazione**: l'heartbeat gira in background; un suo errore non deve mai far fallire il lavoro dell'applicazione né rallentarlo.
- **Log**: un avviso quando una richiesta non arriva dopo tutti i tentativi, con il motivo (mai il segreto, mai la firma completa). Niente log a ogni heartbeat riuscito, al massimo a livello debug.
- **Arresto**: allo spegnimento ordinato smetti di mandare heartbeat; non serve un messaggio finale.

## Esempi di riferimento

Sono implementazioni minime della firma e dell'invio, da adattare allo stile e alle librerie del progetto. Retry e pianificazione restano da aggiungere secondo le regole sopra.

### Shell (cron)

```sh
#!/bin/sh
TS=$(date +%s)
BODY_HASH=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
SIG=$(printf 'v1\nPOST\n/api/v1/heartbeat\n%s\n%s\n%s' "$IRONHEART_KEY_ID" "$TS" "$BODY_HASH" \
  | openssl dgst -sha256 -hmac "$IRONHEART_SECRET" | awk '{print $NF}')
curl -fsS --max-time 10 -X POST "$IRONHEART_URL/api/v1/heartbeat" \
  -H "X-IronHeart-Key: $IRONHEART_KEY_ID" \
  -H "X-IronHeart-Timestamp: $TS" \
  -H "X-IronHeart-Signature: v1=$SIG"
```

### Python (solo libreria standard)

```python
import hashlib, hmac, os, time, urllib.request

EMPTY_BODY_SHA256 = hashlib.sha256(b"").hexdigest()

def sign(secret: str, key_id: str, timestamp: int, body: bytes = b"") -> str:
    canonical = "\n".join(
        ["v1", "POST", "/api/v1/heartbeat", key_id, str(timestamp), hashlib.sha256(body).hexdigest()]
    )
    return hmac.new(secret.encode(), canonical.encode(), hashlib.sha256).hexdigest()

_last_ts = 0

def send_heartbeat() -> bool:
    """True se la richiesta è arrivata (204). Non dice se è valida."""
    global _last_ts
    key_id, secret = os.environ["IRONHEART_KEY_ID"], os.environ["IRONHEART_SECRET"]
    ts = max(int(time.time()), _last_ts + 1)
    _last_ts = ts
    request = urllib.request.Request(
        os.environ["IRONHEART_URL"].rstrip("/") + "/api/v1/heartbeat",
        method="POST",
        data=b"",
        headers={
            "X-IronHeart-Key": key_id,
            "X-IronHeart-Timestamp": str(ts),
            "X-IronHeart-Signature": "v1=" + sign(secret, key_id, ts),
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status == 204
    except OSError:
        return False

assert sign("ihs_test", "abcdefghijkm", 1764000000) == \
    "395ebdbcd9e58a46090ac3c6ca626a0e00b94301076532b944841b3338cd5f01"
```

Nota: `urllib` segue i redirect per default; in un'implementazione completa va disattivato (per esempio con un `HTTPRedirectHandler` che non reindirizza) oppure va usata una libreria che lo permette.

### Node.js (18+, senza dipendenze)

```js
import { createHash, createHmac } from 'node:crypto'

const sha256Hex = (data) => createHash('sha256').update(data).digest('hex')

export function sign(secret, keyId, timestamp, body = '') {
  const canonical = ['v1', 'POST', '/api/v1/heartbeat', keyId, String(timestamp), sha256Hex(body)].join('\n')
  return createHmac('sha256', secret).update(canonical, 'utf8').digest('hex')
}

let lastTs = 0

/** true se la richiesta è arrivata (204). Non dice se è valida. */
export async function sendHeartbeat() {
  const { IRONHEART_URL, IRONHEART_KEY_ID: keyId, IRONHEART_SECRET: secret } = process.env
  const ts = Math.max(Math.floor(Date.now() / 1000), lastTs + 1)
  lastTs = ts
  try {
    const response = await fetch(`${IRONHEART_URL.replace(/\/$/, '')}/api/v1/heartbeat`, {
      method: 'POST',
      redirect: 'manual',
      signal: AbortSignal.timeout(10_000),
      headers: {
        'X-IronHeart-Key': keyId,
        'X-IronHeart-Timestamp': String(ts),
        'X-IronHeart-Signature': `v1=${sign(secret, keyId, ts)}`,
      },
    })
    return response.status === 204
  } catch {
    return false
  }
}

// sign('ihs_test', 'abcdefghijkm', 1764000000)
//   === '395ebdbcd9e58a46090ac3c6ca626a0e00b94301076532b944841b3338cd5f01'
```

### Go

```go
package ironheart

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"strconv"
	"strings"
)

func Sign(secret, keyID string, timestamp int64, body []byte) string {
	bodyHash := sha256.Sum256(body)
	canonical := strings.Join([]string{
		"v1", "POST", "/api/v1/heartbeat", keyID,
		strconv.FormatInt(timestamp, 10), hex.EncodeToString(bodyHash[:]),
	}, "\n")
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(canonical))
	return hex.EncodeToString(mac.Sum(nil))
}

// Sign("ihs_test", "abcdefghijkm", 1764000000, nil)
//   == "395ebdbcd9e58a46090ac3c6ca626a0e00b94301076532b944841b3338cd5f01"
// Per l'invio: http.Client con Timeout di 10 s e CheckRedirect che restituisce
// http.ErrUseLastResponse, così i redirect non vengono seguiti.
```

## Errori comuni

| Sintomo nella dashboard | Causa probabile |
|---|---|
| Nessun «ultimo segnale» e nessun tentativo non valido | Le richieste non arrivano: URL sbagliato, firewall, proxy, redirect seguito su un altro percorso. Controlla i log del client. |
| «Firma errata» | Segreto sbagliato o decodificato invece di usarlo come stringa; `\n` finale nella stringa canonica; `\r\n` invece di `\n`; percorso diverso da `/api/v1/heartbeat`; hash del body calcolato su byte diversi da quelli inviati; key ID con spazi. |
| «Richiesta ripetuta» | Più heartbeat nello stesso secondo, retry che riusa la stessa richiesta, più istanze che condividono la stessa chiave. |
| «Orologio fuori tolleranza» | L'orologio della macchina è sbagliato di più di 5 minuti, oppure il timestamp è in millisecondi invece che in secondi. |
| L'app risulta ferma anche se gli heartbeat partono | L'intervallo del client è più lungo di quello configurato su IronHeart, oppure le richieste falliscono e il client non riprova. |

## Verifica finale

1. Il test automatico della firma con i vettori di prova passa.
2. Con l'applicazione avviata, nella dashboard di IronHeart lo stato dell'applicazione passa a «Attiva» e l'«ultimo segnale» si aggiorna a ogni intervallo.
3. La sezione «Tentativi non validi» resta vuota.
4. Fermando l'applicazione, dopo l'intervallo più la tolleranza arriva la mail «non risponde»; riavviandola arriva la mail «è di nuovo attiva».
