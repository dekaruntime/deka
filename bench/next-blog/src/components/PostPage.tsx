import Link from "next/link";
import { findPost } from "../lib/posts";

export function PostPage({ slug }: { slug: string }) {
  const post = slug ? findPost(slug) : undefined;
  if (!post) {
    return (
      <section>
        <h1 className="page-title">Not found</h1>
        <p>
          <Link href="/">Back to posts</Link>
        </p>
      </section>
    );
  }
  return (
    <article>
      <p className="meta">
        <Link href="/">All posts</Link> · {post.date}
      </p>
      <h1 className="page-title">{post.title}</h1>
      <div className="tags">
        {post.tags.map((tag) => (
          <Link className="tag" key={tag} href={`/tags/${tag}`}>
            {tag}
          </Link>
        ))}
      </div>
      <div className="prose" dangerouslySetInnerHTML={{ __html: post.html }} />
    </article>
  );
}
