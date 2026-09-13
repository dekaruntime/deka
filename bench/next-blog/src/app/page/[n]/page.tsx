import { HomePage } from "../../../components/HomePage";
import { pageNumbers } from "../../../posts.generated";
export const dynamicParams = false;
export function generateStaticParams() { return pageNumbers.map(n => ({ n })); }
export default async function Page({ params }: { params: Promise<{ n: string }> }) {
  const { n } = await params;
  return <HomePage page={Number(n)} />;
}
