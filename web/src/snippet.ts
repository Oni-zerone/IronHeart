/** Script di esempio per un cron: firma HMAC con openssl e invio con curl. */
export function shellSnippet(origin: string, keyId: string, secret = '<il tuo segreto>'): string {
  return `#!/bin/sh
KEY_ID='${keyId}'
SECRET='${secret}'
TS=$(date +%s)
BODY_HASH=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
SIG=$(printf 'v1\\nPOST\\n/api/v1/heartbeat\\n%s\\n%s\\n%s' "$KEY_ID" "$TS" "$BODY_HASH" \\
  | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $NF}')
curl -fsS -X POST ${origin}/api/v1/heartbeat \\
  -H "X-IronHeart-Key: $KEY_ID" \\
  -H "X-IronHeart-Timestamp: $TS" \\
  -H "X-IronHeart-Signature: v1=$SIG"`
}
