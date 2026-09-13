import Link from "next/link";
import type { Post } from "../lib/posts";

export function PostCard({ post }: { post: Post }) {
  return (
    <Link className="card" href={`/posts/${post.slug}`}>
      <span data-bench-edit="card">Read post</span>
      <h2>{post.title}</h2>
      <div className="meta">{post.date}</div>
      <p>{post.excerpt}</p>
      <div className="tags">
        {post.tags.map((tag) => (
          <span className="tag" key={tag}>
            {tag}
          </span>
        ))}
      </div>
    </Link>
  );
}
