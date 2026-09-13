import { copyFileSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PAGE_SIZE, renderMarkdown } from "./markdown.mjs";

const here = dirname(fileURLToPath(import.meta.url));
export const BENCH_ROOT = join(here, "..");

function dsString(s) {
  return JSON.stringify(s);
}

export function loadPosts() {
  const dir = join(BENCH_ROOT, "content/posts");
  const files = readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .sort();
  const posts = files.map((file) => {
    const raw = readFileSync(join(dir, file), "utf8");
    const { meta, html } = renderMarkdown(raw);
    const slug = file.replace(/\.md$/, "");
    return {
      slug,
      title: meta.title || slug,
      date: meta.date || "1970-01-01",
      excerpt: meta.excerpt || "",
      tags: meta.tags || [],
      html,
    };
  });
  posts.sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : a.slug.localeCompare(b.slug)));
  return posts;
}

function allTags(posts) {
  const set = new Set();
  for (const p of posts) for (const t of p.tags) set.add(t);
  return [...set].sort();
}

function emitDs(posts) {
  const tags = allTags(posts);
  const pageCount = Math.max(1, Math.ceil(posts.length / PAGE_SIZE));
  const postStructs = posts
    .map(
      (p) =>
        `Post { slug: ${dsString(p.slug)}, title: ${dsString(p.title)}, date: ${dsString(p.date)}, excerpt: ${dsString(p.excerpt)}, tags: [${p.tags.map(dsString).join(", ")}], html: ${dsString(p.html)} }`,
    )
    .join(", ");
  const postParams = posts.map((p) => `PostParam { slug: ${dsString(p.slug)} }`).join(", ");
  const tagParams = tags.map((t) => `TagParam { tag: ${dsString(t)} }`).join(", ");
  const pageParams = [];
  for (let n = 2; n <= pageCount; n++) pageParams.push(`PageParam { n: ${dsString(String(n))} }`);
  return `struct Post {
  slug: string
  title: string
  date: string
  excerpt: string
  tags: Array<string>
  html: string
}
struct PostParam {
  slug: string
}
struct TagParam {
  tag: string
}
struct PageParam {
  n: string
}

export const PAGE_SIZE: number = ${PAGE_SIZE}
export const posts: Array<Post> = [${postStructs}]
export const postParams: Array<PostParam> = [${postParams}]
export const tagParams: Array<TagParam> = [${tagParams}]
export const pageParams: Array<PageParam> = [${pageParams.join(", ")}]
export const tagNames: Array<string> = [${tags.map(dsString).join(", ")}]

export fn hasTag(tags: Array<string>, tag: string) boolean {
  for (const t of tags) {
    if (t == tag) {
      return true
    }
  }
  return false
}

export fn findPost(slug: string) Post {
  for (const post of posts) {
    if (post.slug == slug) {
      return post
    }
  }
  if (posts.has(0)) {
    return posts[0]
  }
  return Post { slug: "", title: "Missing", date: "", excerpt: "", tags: [], html: "" }
}

export fn postsForTag(tag: string) Array<Post> {
  let out: Array<Post> = []
  for (const post of posts) {
    if (hasTag(post.tags, tag)) {
      out.push(post)
    }
  }
  return out
}

export fn pageCount() number {
  const n = posts.length
  if (n == 0) {
    return 1
  }
  let pages = 0
  for (let i = 0; i < n; i = i + PAGE_SIZE) {
    pages = pages + 1
  }
  return pages
}

export fn pageSlice(page: number) Array<Post> {
  let start = (page - 1) * PAGE_SIZE
  if (start < 0) {
    start = 0
  }
  const end = start + PAGE_SIZE
  let out: Array<Post> = []
  let i = 0
  for (const post of posts) {
    if (i >= start) {
      if (i < end) {
        out.push(post)
      }
    }
    i = i + 1
  }
  return out
}

export fn hrefForPage(page: number) string {
  if (page <= 1) {
    return "/"
  }
  return "/page/" + string(page)
}
`;
}

function emitTs(posts) {
  const tags = allTags(posts);
  const pageCount = Math.max(1, Math.ceil(posts.length / PAGE_SIZE));
  return `export const PAGE_SIZE = ${PAGE_SIZE} as const;
export type Post = {
  slug: string;
  title: string;
  date: string;
  excerpt: string;
  tags: string[];
  html: string;
};
export const posts: Post[] = ${JSON.stringify(posts, null, 2)};
export const tagNames: string[] = ${JSON.stringify(tags)};
export const pageNumbers: string[] = ${JSON.stringify(
    Array.from({ length: Math.max(0, pageCount - 1) }, (_, i) => String(i + 2)),
  )};
`;
}

function copyImages(dest) {
  const src = join(BENCH_ROOT, "content/images");
  mkdirSync(dest, { recursive: true });
  for (const file of readdirSync(src)) {
    copyFileSync(join(src, file), join(dest, file));
  }
}

export function ingest() {
  const posts = loadPosts();
  if (posts.length < 25) {
    throw new Error(`expected 25+ posts, found ${posts.length}`);
  }
  mkdirSync(join(BENCH_ROOT, "deka-blog/src"), { recursive: true });
  mkdirSync(join(BENCH_ROOT, "vite-blog/src"), { recursive: true });
  writeFileSync(join(BENCH_ROOT, "deka-blog/src/posts.generated.ds"), emitDs(posts));
  writeFileSync(join(BENCH_ROOT, "vite-blog/src/posts.generated.ts"), emitTs(posts));
  copyImages(join(BENCH_ROOT, "deka-blog/public/images"));
  copyImages(join(BENCH_ROOT, "vite-blog/public/images"));
  return { posts: posts.length, tags: allTags(posts).length };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const counts = ingest();
  process.stdout.write(`ingested ${counts.posts} posts, ${counts.tags} tags\n`);
}
