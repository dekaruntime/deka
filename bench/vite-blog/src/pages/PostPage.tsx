import { Link, useParams } from "react-router-dom";
import { findPost } from "../lib/posts";

export function PostPage() {
  const { slug } = useParams();
  const post = slug ? findPost(slug) : undefined;
  if (!post) {
    return (
      <section>
        <h1 className="page-title">Not found</h1>
        <p>
          <Link to="/">Back to posts</Link>
        </p>
      </section>
    );
  }
  return (
    <article>
      <p className="meta">
        <Link to="/">All posts</Link> · {post.date}
      </p>
      <h1 className="page-title">{post.title}</h1>
      <div className="tags">
        {post.tags.map((tag) => (
          <Link className="tag" key={tag} to={`/tags/${tag}`}>
            {tag}
          </Link>
        ))}
      </div>
      <div className="prose" dangerouslySetInnerHTML={{ __html: post.html }} />
    </article>
  );
}
