// ui/island-marker — the single definition of the `deka-island` HTML comment
// grammar. ui/server (producer) and ui/client (consumer) both derive their
// marker formatter/parser from this module, so a field can be added, renamed,
// or reordered in exactly one place and the two cannot drift (deka#622
// finding C). Never import ui/server or ui/client here.
//
// Wire shape (one HTML comment):
//   deka-island start:<b64 name> directive:<b64 directive> [props:<b64 json>]
//     [id:<b64 id>] [enc:<b64 nonce||ciphertext>] [cache:<b64 cache-control>]
// paired with an end comment:
//   deka-island end:<b64 name>
// `start` and `directive` are always present; every field after them is
// optional but must appear in the ISLAND_MARKER_FIELDS order. `props` is a
// JSON object serialized by the producer; `enc` is already base64 (the
// producer emits the ciphertext verbatim); the rest are plain UTF-8 strings.
//
// The coarse scanners (crates/http/src/websocket.rs, the HMR patch client)
// only match the literal `deka-island start:` / `deka-island end:` prefixes;
// keep ISLAND_MARKER_TAG stable for them.

export const ISLAND_MARKER_TAG = "deka-island";

// Ordered grammar. Index 0-1 are required and always emitted; the rest are
// optional. This list drives both the consumer's match regex and (by
// contract, enforced in tests) the producer's emission order.
export const ISLAND_MARKER_FIELDS = ["start", "directive", "props", "id", "enc", "cache"];

const B64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_PATTERN = "[A-Za-z0-9+/=]+";

const ISLAND_MARKER_RE = (() => {
  // Fields after `directive` are optional; each optional group's leading
  // space lives INSIDE the group so absent fields leave no dangling space.
  const parts = ISLAND_MARKER_FIELDS.map((field, index) => {
    if (index === 0) return `${field}:(${B64_PATTERN})`;
    if (index === 1) return ` ${field}:(${B64_PATTERN})`;
    return `(?: ${field}:(${B64_PATTERN}))?`;
  });
  return new RegExp(`^${ISLAND_MARKER_TAG} ${parts.join("")}$`);
})();

function utf8Bytes(str) {
  const out = [];
  for (let i = 0; i < str.length; i++) {
    let c = str.charCodeAt(i);
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) {
      i += 1;
      c = 0x10000 + ((c & 0x3ff) << 10) + (str.charCodeAt(i) & 0x3ff);
      out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 0x3f), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
    } else {
      out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
    }
  }
  return out;
}

function encodeB64(str) {
  const bytes = utf8Bytes(String(str));
  let output = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i];
    const b = i + 1 < bytes.length ? bytes[i + 1] : 0;
    const c = i + 2 < bytes.length ? bytes[i + 2] : 0;
    const triple = (a << 16) | (b << 8) | c;
    output += B64_ALPHABET[(triple >> 18) & 63];
    output += B64_ALPHABET[(triple >> 12) & 63];
    output += i + 1 < bytes.length ? B64_ALPHABET[(triple >> 6) & 63] : "=";
    output += i + 2 < bytes.length ? B64_ALPHABET[triple & 63] : "=";
  }
  return output;
}

function decodeB64(value) {
  const raw = String(value || "");
  if (!raw) return "";
  let binary = "";
  try {
    if (typeof atob === "function") binary = atob(raw);
  } catch (_) {
    binary = "";
  }
  if (!binary) {
    const cleaned = raw.replace(/[^A-Za-z0-9+/=]/g, "");
    const bytes = [];
    for (let i = 0; i < cleaned.length; i += 4) {
      const a = B64_ALPHABET.indexOf(cleaned[i]);
      const b = B64_ALPHABET.indexOf(cleaned[i + 1]);
      const c = B64_ALPHABET.indexOf(cleaned[i + 2]);
      const d = B64_ALPHABET.indexOf(cleaned[i + 3]);
      const triple = ((a & 63) << 18) | ((b & 63) << 12) | ((c & 63) << 6) | (d & 63);
      bytes.push((triple >> 16) & 255);
      if (cleaned[i + 2] !== "=") bytes.push((triple >> 8) & 255);
      if (cleaned[i + 3] !== "=") bytes.push(triple & 255);
    }
    binary = String.fromCharCode.apply(null, bytes);
  }
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i) & 255;
  if (typeof TextDecoder === "function") {
    try {
      const dec = new TextDecoder("utf-8");
      // Sandboxed runtimes wrap throwing constructors in Result; unwrap.
      const real = dec && dec.__case === "Ok" ? dec.value : dec;
      return real.decode(bytes);
    } catch (_) {}
  }
  let out = "";
  for (let i = 0; i < bytes.length; i++) {
    const c = bytes[i];
    if (c < 0x80) out += String.fromCharCode(c);
    else if (c < 0xe0 && i + 1 < bytes.length) {
      out += String.fromCharCode(((c & 0x1f) << 6) | (bytes[i + 1] & 0x3f));
      i += 1;
    } else if (c < 0xf0 && i + 2 < bytes.length) {
      out += String.fromCharCode(((c & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f));
      i += 2;
    } else if (i + 3 < bytes.length) {
      const u = ((c & 0x07) << 18) | ((bytes[i + 1] & 0x3f) << 12) | ((bytes[i + 2] & 0x3f) << 6) | (bytes[i + 3] & 0x3f);
      const v = u - 0x10000;
      out += String.fromCharCode(0xd800 + (v >> 10), 0xdc00 + (v & 0x3ff));
      i += 3;
    }
  }
  return out;
}

/// Format the start marker's inner comment text (without the `<!--`/`-->`
/// delimiters). Fields are emitted in ISLAND_MARKER_FIELDS order; optional
/// fields are skipped when the caller passes no value for them.
///
/// `propsJson` must already be serialized JSON (the producer owns prop
/// serialization policy); `enc` must already be base64 (the producer emits
/// the ciphertext verbatim); `id` and `cache` are plain strings.
export function formatIslandStart(marker) {
  const name = String(marker.name || "Anonymous");
  const directive = String(marker.directive || "load");
  let out = `${ISLAND_MARKER_TAG} start:${encodeB64(name)} directive:${encodeB64(directive)}`;
  // Optional fields, in ISLAND_MARKER_FIELDS order.
  const extras = [];
  if (marker.propsJson != null && marker.propsJson !== "") {
    extras.push(`props:${encodeB64(String(marker.propsJson))}`);
  }
  if (marker.id != null && marker.id !== "") extras.push(`id:${encodeB64(String(marker.id))}`);
  if (marker.enc) extras.push(`enc:${marker.enc}`);
  if (marker.cache != null && marker.cache !== "") extras.push(`cache:${encodeB64(String(marker.cache))}`);
  if (extras.length > 0) out += ` ${extras.join(" ")}`;
  return out;
}

/// Format the paired end marker's inner comment text.
export function formatIslandEnd(name) {
  return `${ISLAND_MARKER_TAG} end:${encodeB64(String(name || "Anonymous"))}`;
}

/// Parse a comment node's text. Returns null when the text is not a
/// `deka-island` marker — the caller must treat null as "not an island" and
/// skip it, never as an error.
export function parseIslandMarker(text) {
  const match = String(text || "").trim().match(ISLAND_MARKER_RE);
  if (!match) return null;
  const fields = {};
  ISLAND_MARKER_FIELDS.forEach((field, index) => {
    fields[field] = match[index + 1] || "";
  });
  let props = {};
  if (fields.props) {
    try {
      props = JSON.parse(decodeB64(fields.props) || "{}") || {};
    } catch (_) {
      props = {};
    }
  }
  return {
    name: decodeB64(fields.start),
    directive: decodeB64(fields.directive) || "load",
    props,
    id: fields.id ? decodeB64(fields.id) : "",
    enc: fields.enc || "",
    cache: fields.cache ? decodeB64(fields.cache) : "",
  };
}
