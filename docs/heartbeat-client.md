# Prompt: integrating the IronHeart heartbeat into an application

Copy this document as is to whoever has to implement the client (a person or an AI assistant), together with the code of the application to integrate. It contains everything needed: there is no need to read IronHeart's code.

---

## Task

Add a **heartbeat to IronHeart** to this application: a signed HTTP request, sent at regular intervals, that proves the application is alive. If IronHeart does not receive a valid heartbeat within the interval configured for the application, it sends an alert by email.

The heartbeat must be sent **from the point that really proves the application is working**: for a scheduled job, at the end of a successful run; for a service, from a periodic task of the main process (not from a separate process that would stay alive even if the service is stuck).

## Configuration

Three values, read from the application's configuration (environment variables or a secret manager), **never hard-coded**:

| Variable | Example | Notes |
|---|---|---|
| `IRONHEART_URL` | `https://heartbeat.example.com` | Only the origin, no trailing `/` and no path. |
| `IRONHEART_KEY_ID` | `q88gwdjtcfdr` | Public: identifies the application. |
| `IRONHEART_SECRET` | `ihs_…` | Secret. Never log it, display it or send it over the network. |

The key ID and the secret come from the IronHeart dashboard when the application is created (the secret is shown only once). If any of the three values is missing, the heartbeat must be disabled with a warning in the log at startup, without blocking the application.

## Protocol

### Request

```
POST {IRONHEART_URL}/api/v1/heartbeat
X-IronHeart-Key:       {IRONHEART_KEY_ID}
X-IronHeart-Timestamp: {timestamp}
X-IronHeart-Signature: v1={signature}
```

- `timestamp`: Unix seconds (UTC) as a decimal integer, for example `1764000000`.
- Body: **empty** (recommended). If you send one, at most 1 KB, and it must be signed with exactly the bytes sent.
- `Content-Type` is not required.

### Signature

1. Compute the SHA-256 of the body as lowercase hex. With an empty body it is always:
   `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
2. Build the canonical string, with `\n` (LF, a single character) as separator and **no** trailing `\n`:
   ```
   v1\nPOST\n/api/v1/heartbeat\n{key_id}\n{timestamp}\n{sha256_body_hex}
   ```
3. Compute the HMAC-SHA256 of the canonical string (UTF-8 encoded) using as key **the secret string as is**, in UTF-8, including the `ihs_` prefix (do not base64-decode it).
4. The header is `v1=` followed by the HMAC as lowercase hex (64 characters).

### Test vectors

Use them in an automated test of the client: if they do not match, the signature is wrong.

| | Value |
|---|---|
| key ID | `abcdefghijkm` |
| secret | `ihs_test` |
| timestamp | `1764000000` |
| signature, empty body | `395ebdbcd9e58a46090ac3c6ca626a0e00b94301076532b944841b3338cd5f01` |
| body | `{"status":"ok"}` (15 bytes, no spaces or newlines) |
| SHA-256 of the body | `a29ee2b15c494311c52521766e44af56a3ad2248e7a8ab465e5206463c13d288` |
| signature, with that body | `f9cefe637f8e3e4b9688b79f948bf544702d7b76e983f927d6293749d20fe9b3` |

### Response

**IronHeart always answers `204 No Content`**, even if the signature is wrong, the key does not exist or the timestamp is out of tolerance: this is a security choice, so that whoever tries to forge a ping cannot tell whether it worked. Therefore:

- a `204` only means the request arrived, **not** that it is valid;
- any other response (`5xx`, redirects, network errors, timeouts) means the request did not reach the service;
- validity can only be checked in the IronHeart dashboard: "Last signal" must update and the "Invalid attempts" section must stay empty.

## Rules enforced by the server

A request is valid only if:

1. the signature matches;
2. the timestamp is within **±5 minutes** of the server's clock;
3. the timestamp is **strictly greater** than that of the last heartbeat accepted for the same key.

Consequences for the client:

- **At most one heartbeat per second** per key: with two requests in the same second, the second is discarded (and counted as "Replayed").
- **Never reuse a request**: on every attempt, retries included, recompute timestamp and signature. The timestamp must always increase: if the new value is not greater than the last one sent, use the last one + 1.
- **The machine's clock must be synchronized** (NTP). With more than 5 minutes of skew every heartbeat is discarded as "Clock skew".
- A key belongs to **a single instance** of the application. If several copies run (multiple replicas, multiple servers), each must have its own application and its own key on IronHeart: copies sharing a key discard each other's pings.

## Required client behavior

- **Interval**: send the heartbeat at an interval **shorter** than the one configured on IronHeart, for example half of it, so a single lost ping is not enough to trigger the alert. Make it configurable (`IRONHEART_INTERVAL_SECS`, if needed).
- **Timeout**: 10 seconds per request.
- **Retries**: if the request does not arrive (network error, timeout, response other than `204`), retry at most 2–3 times with increasing waits (for example 2 s, 5 s, 10 s), recomputing timestamp and signature on every attempt; then give up until the next cycle. Do not retry after a `204`.
- **Redirects**: do not follow them. The signed URL is fixed; a redirect means a wrong configuration.
- **HTTPS** is mandatory in production (HTTP only to `localhost` in development).
- **Do not block the application**: the heartbeat runs in the background; its errors must never make the application's work fail or slow it down.
- **Logging**: a warning when a request does not arrive after all attempts, with the reason (never the secret, never the full signature). No log for every successful heartbeat, at most at debug level.
- **Shutdown**: on a graceful shutdown stop sending heartbeats; no final message is needed.

## Reference examples

These are minimal implementations of the signature and of the sending, to be adapted to the project's style and libraries. Retries and scheduling are still to be added according to the rules above.

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

### Python (standard library only)

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
    """True if the request arrived (204). Does not tell whether it is valid."""
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

Note: `urllib` follows redirects by default; a complete implementation must disable that (for example with an `HTTPRedirectHandler` that does not redirect) or use a library that allows it.

### Node.js (18+, no dependencies)

```js
import { createHash, createHmac } from 'node:crypto'

const sha256Hex = (data) => createHash('sha256').update(data).digest('hex')

export function sign(secret, keyId, timestamp, body = '') {
  const canonical = ['v1', 'POST', '/api/v1/heartbeat', keyId, String(timestamp), sha256Hex(body)].join('\n')
  return createHmac('sha256', secret).update(canonical, 'utf8').digest('hex')
}

let lastTs = 0

/** true if the request arrived (204). Does not tell whether it is valid. */
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
// To send it: an http.Client with a 10 s Timeout and a CheckRedirect that returns
// http.ErrUseLastResponse, so redirects are not followed.
```

## Common errors

| Symptom in the dashboard | Likely cause |
|---|---|
| No "Last signal" and no invalid attempts | Requests do not arrive: wrong URL, firewall, proxy, redirect followed to another path. Check the client's logs. |
| "Bad signature" | Wrong secret, or decoded instead of used as a string; trailing `\n` in the canonical string; `\r\n` instead of `\n`; path other than `/api/v1/heartbeat`; body hash computed on bytes other than those sent; key ID with spaces. |
| "Replayed" | Several heartbeats in the same second, a retry reusing the same request, several instances sharing the same key. |
| "Clock skew" | The machine's clock is off by more than 5 minutes, or the timestamp is in milliseconds instead of seconds. |
| The app is reported down even though heartbeats are sent | The client's interval is longer than the one configured on IronHeart, or requests fail and the client does not retry. |

## Final check

1. The automated signature test with the test vectors passes.
2. With the application running, in the IronHeart dashboard the application's status becomes "Up" and "Last signal" updates at every interval.
3. The "Invalid attempts" section stays empty.
4. When the application is stopped, after the interval plus the grace period the "not responding" email arrives; when it is restarted, the "back up" email arrives.
