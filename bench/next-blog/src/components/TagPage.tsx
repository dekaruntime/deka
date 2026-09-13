import Link from "next/link";
import { PostCard } from "../components/PostCard";
import { postsForTag } from "../lib/posts";

export function TagPage({ tag }: { tag: string }) {
  const items = postsForTag(tag);
  return (
    <div>
      <p className="meta">
        <Link href="/">All posts</Link>
      </p>
      <h1 className="page-title">Tag: {tag}</h1>
      <div className="grid">
        {items.map((post) => (
          <PostCard key={post.slug} post={post} />
        ))}
      </div>
    </div>
  );
}
