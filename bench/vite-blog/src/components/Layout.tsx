import { Link, Outlet } from "react-router-dom";
import { NewsletterSignup } from "./Newsletter";
import { ThemeProvider, ThemeToggle } from "./Theme";

export function Layout() {
  return (
    <ThemeProvider>
      <header className="site-header">
        <nav className="nav">
          <Link className="brand" to="/">
            Signal Path
          </Link>
          <div className="nav-links">
            <Link to="/">Posts</Link>
            <Link to="/about">About</Link>
            <ThemeToggle />
          </div>
        </nav>
      </header>
      <div className="wrap">
        <Outlet />
      </div>
      <footer className="site-footer">
        <div className="footer-inner">
          <p>A fair-play subject app. Theme and newsletter are the only interactive widgets.</p>
          <NewsletterSignup />
        </div>
      </footer>
    </ThemeProvider>
  );
}
