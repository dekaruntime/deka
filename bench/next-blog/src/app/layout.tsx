import type { ReactNode } from "react";
import Link from "next/link";
import { ThemeProvider, ThemeToggle } from "../components/Theme";
import { NewsletterSignup } from "../components/Newsletter";
import "./style.css";

export const metadata = { title: "Signal Path", description: "The fair-play 28-post benchmark blog" };

export default function RootLayout({ children }: { children: ReactNode }) {
  return <html lang="en" suppressHydrationWarning><body><ThemeProvider>
    <header className="site-header"><nav className="nav">
      <Link className="brand" href="/">Signal Path</Link>
      <div className="nav-links"><Link href="/">Posts</Link><Link href="/about">About</Link><ThemeToggle /></div>
    </nav></header>
    <div className="wrap">{children}</div>
    <footer className="site-footer"><div className="footer-inner">
      <p>A fair-play subject app. Theme and newsletter are the only interactive widgets.</p>
      <NewsletterSignup />
    </div></footer>
  </ThemeProvider></body></html>;
}
