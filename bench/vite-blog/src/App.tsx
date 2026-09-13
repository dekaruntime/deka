import { lazy, Suspense } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "./components/Layout";
import { HomePage } from "./pages/HomePage";

const PostPage = lazy(() => import("./pages/PostPage").then((m) => ({ default: m.PostPage })));
const TagPage = lazy(() => import("./pages/TagPage").then((m) => ({ default: m.TagPage })));
const AboutPage = lazy(() => import("./pages/AboutPage").then((m) => ({ default: m.AboutPage })));

export function App() {
  return (
    <Suspense fallback={<p className="wrap lede">Loading…</p>}>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<HomePage page={1} />} />
          <Route path="page/:n" element={<HomePage />} />
          <Route path="posts/:slug" element={<PostPage />} />
          <Route path="tags/:tag" element={<TagPage />} />
          <Route path="about" element={<AboutPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </Suspense>
  );
}
