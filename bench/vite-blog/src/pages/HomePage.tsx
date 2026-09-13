import { Link, useParams } from "react-router-dom";
import { PostCard } from "../components/PostCard";
import { hrefForPage, pageCount, pageSlice, parsePage } from "../lib/posts";
import { tagNames } from "../posts.generated";

export function HomePage({ page: pageProp }: { page?: number }) {
  const params = useParams();
  const page = pageProp ?? parsePage(params.n);
  const items = pageSlice(page);
  const total = pageCount();
  return (
    <div>
      <div className="hero">
        <h1>Signal Path</h1>
        <p className="lede">
          A 28-post blog used as a fair-play subject app. Same markdown as the deka column.
        </p>
        <div className="tags">
          {tagNames.map((tag) => (
            <Link className="tag" key={tag} to={`/tags/${tag}`}>
              {tag}
            </Link>
          ))}
        </div>
      </div>
      <div className="grid">
        {items.map((post) => (
          <PostCard key={post.slug} post={post} />
        ))}
      </div>
      <div className="pager">
        {page > 1 ? <Link to={hrefForPage(page - 1)}>Previous</Link> : <span />}
        {page < total ? <Link to={hrefForPage(page + 1)}>Next</Link> : <span />}
      </div>
    </div>
  );
}
