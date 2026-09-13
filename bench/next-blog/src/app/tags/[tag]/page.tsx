import { TagPage } from "../../../components/TagPage";
import { tagNames } from "../../../posts.generated";
export const dynamicParams = false;
export function generateStaticParams() { return tagNames.map(tag => ({ tag })); }
export default async function Page({ params }: { params: Promise<{ tag: string }> }) {
  const { tag } = await params;
  return <TagPage tag={tag} />;
}
