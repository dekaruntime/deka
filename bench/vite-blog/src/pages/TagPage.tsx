import { Link, useParams } from "react-router-dom";
import { PostCard } from "../components/PostCard";
import { postsForTag } from "../lib/posts";

export function TagPage() {
  const { tag = "" } = useParams();
  const items = postsForTag(tag);
  return (
    <div>
      <p className="meta">
        <Link to="/">All posts</Link>
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
