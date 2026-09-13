import { Link } from "react-router-dom";
import type { Post } from "../lib/posts";

export function PostCard({ post }: { post: Post }) {
  return (
    <Link className="card" to={`/posts/${post.slug}`}>
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
