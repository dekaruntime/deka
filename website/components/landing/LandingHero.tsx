import Link from 'next/link'
import styles from '@/components/landing/landing.module.css'

export function LandingHero() {
  return (
    <section className={styles.hero}>
      <div className={styles.heroInner}>
        <div className={styles.heroCopy}>
          <span className={styles.heroEyebrow}>Deka runtime</span>
          <h1 className={styles.heroTitle}>A simpler runtime for serious apps.</h1>
          <p className={styles.heroSubtitle}>
            Apple-like clarity for your infrastructure. Serve, run, build, compile, and deploy with a
            runtime that stays fast and predictable.
          </p>
          <p className={styles.heroSubtitle}>
            PHP-compatible. PHPX-modern. One runtime.
          </p>
          <div className={styles.heroActions}>
            <Link className={styles.primaryAction} href="/install">
              Get started
            </Link>
            <Link className={styles.secondaryAction} href="/help">
              Read the docs
            </Link>
          </div>
        </div>
        <div className={`${styles.heroCard} ${styles.reveal}`}>
          <div className={styles.heroCardHeader}>
            <span>Performance</span>
            <span className={styles.heroCardTag}>Multi-core</span>
          </div>
          <h2>2,100+ RPS on commodity hardware.</h2>
          <p>
            Deka isolates scale linearly across cores while keeping latency stable. Build for edge-grade
            throughput without reworking your stack.
          </p>
          <div className={styles.heroStats}>
            <div>
              <span>p50</span>
              <strong>76ms</strong>
            </div>
            <div>
              <span>p95</span>
              <strong>92ms</strong>
            </div>
            <div>
              <span>Warm start</span>
              <strong>0.3ms</strong>
            </div>
          </div>
        </div>
      </div>
    </section>
  )
}
