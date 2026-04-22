# `@deka/http`

Outbound HTTP/1.1 and HTTP/2 (ALPN h2), streaming bodies, opt-in cookie
jar, and a WebSocket client. Modelled after Go's `net/http`. Backed by
`reqwest` + `tokio-tungstenite` in the Rust runtime.

Every network call is gated by the tenant's `deka.json` `security.allow.net`
allowlist. Wildcard DNS labels are supported (`*.squareup.com`). Requests
to hosts not in the allowlist fail closed with
`HttpError { kind: 'host_not_allowed', ... }`.

## 1. One-liner

```phpx
import { http_get, http_post } from 'http'
import { result_is_ok } from 'core/result'

$resp = http_get('https://httpbin.org/get', {
    'User-Agent': 'deka/1.0',
})
if (result_is_ok($resp)) {
    $body = $resp->value->body
}
```

```phpx
$resp = http_post(
    'https://api.stripe.com/v1/charges',
    'application/x-www-form-urlencoded',
    'amount=1000&currency=usd',
    { 'Authorization': 'Bearer sk_test_...' }
)
```

## 2. Client — persistent config + optional cookie jar

```phpx
import { http_client } from 'http'

$client_res = http_client({
    timeout_ms: 30000,
    max_redirects: 5,
    cookie_jar: true,  // opt-in; default off
})
$client = $client_res->value

$resp = $client->do({
    method: 'POST',
    url: 'https://api.example.com/login',
    headers: { 'Content-Type': 'application/json' },
    body: '{"user":"a","pass":"b"}',
})

// Later requests on the same client carry cookies set by /login.
$next = $client->do({
    method: 'GET',
    url: 'https://api.example.com/me',
})

// Inspect what cookies the jar would send to a URL.
$seen = $client->jar('https://api.example.com/')

$client->close()
```

## 3. Request struct — full control

```phpx
import { http_request } from 'http'

$resp = http_request({
    method: 'POST',
    url: 'https://api.stripe.com/v1/charges',
    headers: {
        'Authorization': 'Bearer sk_test_...',
        'Idempotency-Key': 'req_abc123',
    },
    body: $form_body,
    timeout_ms: 10000,
})
```

### Streaming request body

```phpx
import { http_req_stream_new, http_req_stream_write, http_req_stream_end, http_request } from 'http'

$stream = http_req_stream_new()
$handle = $stream['stream_handle']

// A producer runs alongside the request and pushes chunks as they arrive.
http_req_stream_write($handle, $first_chunk)
http_req_stream_write($handle, $second_chunk)
http_req_stream_end($handle)

$resp = http_request({
    method: 'POST',
    url: 'https://example.com/upload',
    body_stream_handle: $handle,
    headers: { 'Content-Type': 'application/octet-stream' },
})
```

### Streaming response body

```phpx
import { http_request, http_stream_read, http_stream_close } from 'http'

$resp_res = http_request({
    method: 'GET',
    url: 'https://example.com/big.tar',
    stream_response: true,
})
$resp = $resp_res->value

while (true) {
    $chunk = http_stream_read($resp->stream_handle, 10000)
    if ($chunk['done']) { break }
    if (!$chunk['ok']) { break }
    // $chunk['chunk'] is an array of byte values (0..255)
}
http_stream_close($resp->stream_handle)
```

## 4. Pluggable Transport — tests + middleware

Any function that takes a request map and returns
`Result<HttpResponse, HttpError>` is a valid transport. Pass it as
`transport` on a client or directly on `http_request`.

```phpx
import { http_request } from 'http'
import { result_ok } from 'core/result'

$mock = function ($req) {
    // Unit-test friendly: no network, deterministic response.
    return result_ok({
        status: 200,
        headers: { 'content-type': 'application/json' },
        body: '{"mocked":true}',
        version: 'HTTP/1.1',
        final_url: $req['url'],
        streamed: false,
        stream_handle: null,
        body_bytes: null,
    })
}

$resp = http_request({
    method: 'GET',
    url: 'https://example.com/anything',
    transport: $mock,
})
```

## WebSocket (RFC 6455)

`ws://` and `wss://`. Auto-pong keepalive. Graceful close handshake.
Frame fragmentation reassembled internally — consumers see whole
messages.

```phpx
import { ws_connect } from 'http'

$ws_res = ws_connect('wss://echo.websocket.events')
$ws = $ws_res->value

$ws->send_text('hello')
$frame = $ws->recv(5000)
// $frame['kind'] is one of: 'text', 'binary', 'ping', 'pong', 'close'
// text → $frame['text'], binary → $frame['bytes'] (array of 0..255)

$ws->ping()
$ws->close(1000, 'bye')
```

HTTP/2-bootstrapped WebSocket (RFC 8441) is not exposed separately — we
negotiate h2 via ALPN on every `https://` request and fall back to h1
for the Upgrade handshake when the server doesn't speak h2 for WS.
Stripe's `wss://` endpoints use h1 Upgrade today; `reqwest-websocket`
would be the path to h2 WS when needed. Filed as a follow-up if the
gap becomes real.

## Capability gate (`deka.json`)

```json
{
  "security": {
    "allow": {
      "net": [
        "api.stripe.com",
        "*.squareup.com",
        "echo.websocket.events"
      ]
    }
  }
}
```

- Exact hostname matches are preferred.
- `*.example.com` matches `foo.example.com` and `a.b.example.com`, but
  **not** the bare `example.com` — add both if you need both.
- Cookie jars and streaming both run after the capability check, so
  cookies cannot leak to disallowed hosts.

## Error shape

```
HttpError = {
    kind: 'timeout'
        | 'connect_refused'
        | 'tls_error'
        | 'dns_error'
        | 'too_many_redirects'
        | 'invalid_url'
        | 'body_too_large'
        | 'host_not_allowed'
        | 'transport_error'
        | 'ws_connect_failed'
        | 'ws_send_failed'
        | 'ws_recv_failed'
        | 'frame_too_large',
    message: string
}
```

No throws. No nulls. Check `$resp->ok` / use `result_is_ok($resp)`.
