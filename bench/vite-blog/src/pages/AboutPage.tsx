export function AboutPage() {
  return (
    <article className="prose">
      <h1 className="page-title">About this blog</h1>
      <p>
        Signal Path is the subject app for the deka-bench comparison: one blog, identical markdown,
        implemented in deka and in Vite + React 19. Next.js is a later lane.
      </p>
      <p>
        The chrome is static. The theme toggle and the newsletter form are the only interactive
        widgets. Everything else is HTML the compiler already knew.
      </p>
      <p>
        Fair play is the product. Versions are pinned, the runner is one command, and empty cells in
        the results table stay empty until the owning lane fills them.
      </p>
    </article>
  );
}
