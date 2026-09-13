/** Zero-dep markdown → HTML for the shared bench ingest. Not timed. */

const KEYWORDS = {
  js: "break catch class const continue default else export from function if import let return switch throw try typeof var while async await of in new this",
  ts: "break catch class const continue default else export from function if import let return switch throw try typeof var while async await of in new this interface type",
  ds: "break const else export fn for if import let match return struct interface opaque summon total unsafe useState useEffect useContext createContext",
  rs: "break const crate else enum fn for if impl let match mod mut pub return struct use where async await",
  bash: "if then else fi for in do done echo export return rm cd ls cat curl",
  css: "important root color-scheme display margin padding border font-size",
  json: "true false null",
  md: "title date tags excerpt",
  txt: "",
};

function escapeHtml(s) {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function highlight(code, lang) {
  const keys = (KEYWORDS[lang] || KEYWORDS.js || "").split(/\s+/).filter(Boolean);
  const keySet = new Set(keys);
  const re =
    /("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`)|(\/\/[^\n]*|#[^\n]*|\/\*[\s\S]*?\*\/)|(\b\d+(?:\.\d+)?\b)|([A-Za-z_][A-Za-z0-9_-]*)|(\s+)|([^])/g;
  let out = "";
  let m;
  while ((m = re.exec(code))) {
    if (m[1]) out += `<span class="tok-str">${escapeHtml(m[1])}</span>`;
    else if (m[2]) out += `<span class="tok-com">${escapeHtml(m[2])}</span>`;
    else if (m[3]) out += `<span class="tok-num">${escapeHtml(m[3])}</span>`;
    else if (m[4]) {
      const cls = keySet.has(m[4]) ? "tok-kw" : "tok-id";
      out += `<span class="${cls}">${escapeHtml(m[4])}</span>`;
    } else out += escapeHtml(m[5] || m[6] || "");
  }
  return out;
}

function inline(s) {
  s = escapeHtml(s);
  s = s.replace(/`([^`]+)`/g, "<code>$1</code>");
  s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>');
  return s;
}

function parseFrontMatter(raw) {
  if (!raw.startsWith("---\n")) return { meta: {}, body: raw };
  const end = raw.indexOf("\n---\n", 4);
  if (end < 0) return { meta: {}, body: raw };
  const fm = raw.slice(4, end);
  const body = raw.slice(end + 5);
  const meta = {};
  for (const line of fm.split("\n")) {
    const i = line.indexOf(":");
    if (i < 0) continue;
    const key = line.slice(0, i).trim();
    let val = line.slice(i + 1).trim();
    if (key === "tags") {
      meta.tags = val
        .replace(/^\[/, "")
        .replace(/\]$/, "")
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);
    } else {
      meta[key] = val;
    }
  }
  return { meta, body };
}

export function renderMarkdown(raw) {
  const { meta, body } = parseFrontMatter(raw);
  const lines = body.replace(/\r\n/g, "\n").split("\n");
  const html = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith("```")) {
      const lang = line.slice(3).trim() || "txt";
      i += 1;
      const buf = [];
      while (i < lines.length && !lines[i].startsWith("```")) {
        buf.push(lines[i]);
        i += 1;
      }
      i += 1;
      html.push(
        `<pre class="code"><code class="language-${escapeHtml(lang)}">${highlight(
          buf.join("\n"),
          lang,
        )}</code></pre>`,
      );
      continue;
    }
    if (line.startsWith("| ") && i + 1 < lines.length && /^\|[\s:|-]+\|$/.test(lines[i + 1])) {
      const rows = [];
      while (i < lines.length && lines[i].startsWith("|")) {
        rows.push(lines[i]);
        i += 1;
      }
      const parseRow = (r) =>
        r
          .split("|")
          .slice(1, -1)
          .map((c) => inline(c.trim()));
      const head = parseRow(rows[0]);
      const bodyRows = rows.slice(2).map(parseRow);
      html.push(
        "<table><thead><tr>" +
          head.map((c) => `<th>${c}</th>`).join("") +
          "</tr></thead><tbody>" +
          bodyRows.map((r) => "<tr>" + r.map((c) => `<td>${c}</td>`).join("") + "</tr>").join("") +
          "</tbody></table>",
      );
      continue;
    }
    if (line.startsWith("![") && line.includes("](")) {
      const m = line.match(/^!\[([^\]]*)\]\(([^)]+)\)/);
      if (m) {
        html.push(`<p><img src="${escapeHtml(m[2])}" alt="${escapeHtml(m[1])}" /></p>`);
        i += 1;
        continue;
      }
    }
    if (line.startsWith("## ")) {
      html.push(`<h2>${inline(line.slice(3))}</h2>`);
      i += 1;
      continue;
    }
    if (line.startsWith("# ")) {
      html.push(`<h1>${inline(line.slice(2))}</h1>`);
      i += 1;
      continue;
    }
    if (line.startsWith("- ")) {
      const items = [];
      while (i < lines.length && lines[i].startsWith("- ")) {
        items.push(`<li>${inline(lines[i].slice(2))}</li>`);
        i += 1;
      }
      html.push(`<ul>${items.join("")}</ul>`);
      continue;
    }
    if (/^\d+\. /.test(line)) {
      const items = [];
      while (i < lines.length && /^\d+\. /.test(lines[i])) {
        items.push(`<li>${inline(lines[i].replace(/^\d+\. /, ""))}</li>`);
        i += 1;
      }
      html.push(`<ol>${items.join("")}</ol>`);
      continue;
    }
    if (line.trim() === "") {
      i += 1;
      continue;
    }
    const buf = [line];
    i += 1;
    while (i < lines.length && lines[i].trim() !== "" && !lines[i].startsWith("#") && !lines[i].startsWith("```") && !lines[i].startsWith("| ") && !lines[i].startsWith("- ") && !/^\d+\. /.test(lines[i]) && !lines[i].startsWith("![")) {
      buf.push(lines[i]);
      i += 1;
    }
    html.push(`<p>${inline(buf.join(" "))}</p>`);
  }
  return { meta, html: html.join("\n") };
}

export const PAGE_SIZE = 6;
