import { PostPage } from "../../../components/PostPage";
import { posts } from "../../../posts.generated";
export const dynamicParams = false;
export function generateStaticParams() { return posts.map(({ slug }) => ({ slug })); }
export default async function Page({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  return <PostPage slug={slug} />;
}
