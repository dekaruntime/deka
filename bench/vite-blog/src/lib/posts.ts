import { PAGE_SIZE, posts, type Post } from "../posts.generated";

export { PAGE_SIZE, posts };
export type { Post };

export function findPost(slug: string): Post | undefined {
  return posts.find((post) => post.slug === slug);
}

export function postsForTag(tag: string): Post[] {
  return posts.filter((post) => post.tags.includes(tag));
}

export function pageCount(): number {
  return Math.max(1, Math.ceil(posts.length / PAGE_SIZE));
}

export function pageSlice(page: number): Post[] {
  const start = Math.max(0, (page - 1) * PAGE_SIZE);
  return posts.slice(start, start + PAGE_SIZE);
}

export function hrefForPage(page: number): string {
  return page <= 1 ? "/" : `/page/${page}`;
}

export function parsePage(n: string | undefined): number {
  const value = Number(n);
  return Number.isFinite(value) && value >= 1 ? Math.floor(value) : 1;
}
