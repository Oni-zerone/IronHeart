/** Example cron script: HMAC signature with openssl, sent with curl. */
export function shellSnippet(origin: string, keyId: string, secret = '<your secret>'): string {
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

const escapeHtml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

/** The same script as HTML, with variables and headers highlighted. The text is escaped first. */
export function highlightSnippet(script: string): string {
  return escapeHtml(script)
    .replace(/^(#!.*)$/m, '<span class="sh-comment">$1</span>')
    .replace(/\b(KEY_ID|SECRET|TS|BODY_HASH|SIG)=/g, '<span class="sh-key">$1</span>=')
    .replace(/(X-IronHeart-[A-Za-z]+)/g, '<span class="sh-key">$1</span>')
}
