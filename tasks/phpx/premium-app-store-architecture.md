# Premium App Store Architecture

Issue: tana/deka#57

## Task Status

- [x] Design the Premium app-store architecture and implementation phases.

## Summary

Premium apps are PHPX packages with an install-time product contract. They use
the same Linkhash artifact, integrity, and module-resolution machinery as normal
packages, but add store-admin discovery, merchant approval, tenant-scoped app
state, and a stricter capability review path.

The app-store boundary should stay out of PHPX runtime semantics. PHPX still
imports packages normally, and app code still runs inside the tenant isolate.
The app layer decides which package may be installed, which lifecycle hooks run,
which UI extension points are exposed, and which declared capabilities a merchant
has approved.

## Package Shape

Apps are published to reserved Linkhash scopes:

- `@tana-apps/*` for first-party apps.
- `@apps/*` for reviewed marketplace apps.
- Partner or vendor scopes can be added later, but should still publish an app
  manifest and pass the same review checks.

An app is a PHPX package with extra `deka.json` metadata:

```json
{
  "name": "@tana-apps/mail",
  "version": "1.2.0",
  "type": "phpx-app",
  "main": "index.phpx",
  "app": {
    "displayName": "Mail",
    "category": "operations",
    "plan": "premium",
    "extensionPoints": [
      "store-admin.settings",
      "checkout.after_order_created"
    ],
    "lifecycle": {
      "install": "hooks.install",
      "upgrade": "hooks.upgrade",
      "uninstall": "hooks.uninstall"
    },
    "settingsSchema": "./settings.schema.json"
  },
  "security": {
    "allow": {
      "net": ["api.mailgun.net:443"],
      "env": ["MAILGUN_API_KEY"],
      "db": ["app:@tana-apps/mail"]
    }
  }
}
```

Normal packages may expose reusable code. Apps may additionally declare:

- Catalog metadata: name, category, icon/media, pricing label, support URL.
- Extension points: where the platform may call or render the app.
- Lifecycle hooks: install, upgrade, disable, uninstall.
- Settings schema: merchant-editable configuration.
- Capability requirements: host operations the app needs.

The runtime should reject `type: "phpx-app"` packages that are not installed
through the app installer path. That keeps direct `deka install` useful for
development without bypassing merchant approval in production.

## Discovery

Discovery belongs in store-admin and reads a curated catalog API, not raw package
search. The catalog service can be backed by Linkhash package metadata, but it
must only expose reviewed app releases.

Catalog records should include:

- Canonical package coordinate and immutable release hash.
- Current reviewed version.
- Plan requirement, pricing label, and first-party/third-party marker.
- Declared capabilities and human-readable permission reasons.
- Screenshots, docs URL, support URL, and changelog.
- Compatibility constraints, such as minimum Deka runtime version.

Store-admin installation should show the capability approval screen before any
artifact is written into the tenant project.

## Pricing

MVP pricing should be simple:

- Premium unlocks installation of included first-party and reviewed free apps.
- Paid add-ons can be modeled later as a catalog billing flag, not as a runtime
  concern.
- Linkhash package access may still be private, but entitlement is checked by the
  app catalog/install API before issuing the artifact download.

Runtime execution should never decide billing. It receives an already-installed
and approved app set for the tenant.

## Permissions

Apps reuse the existing Deno-inspired `security.allow` capability groups in
`deka.json`, with two app-specific rules:

1. Requested capabilities must be declared by the package release and approved by
   the merchant at install or upgrade time.
2. App data access should use tenant and app scoped targets, for example
   `db: ["app:@tana-apps/mail"]`, rather than broad project database access.

Effective policy for tenant execution is:

1. Platform deny defaults.
2. Tenant project policy.
3. Installed app approved capabilities.
4. Platform hard denies.

Denies always win. Approval is stored per tenant, package coordinate, version
range, and capability set. If a new version asks for broader capabilities, the
upgrade pauses for merchant approval.

## Lifecycle

The installer records each installed app in tenant metadata:

- package coordinate
- installed version and lockfile hashes
- enabled/disabled status
- approved capabilities
- installed extension points
- app settings
- last lifecycle status

Lifecycle hooks run as platform-managed calls inside the tenant isolate with the
app's approved capability set. Hooks return errors as values; a failing install
does not partially enable the app.

Recommended hook contract:

```php
function install(AppInstallContext $ctx): Result<AppInstallSummary, AppError>
function upgrade(AppUpgradeContext $ctx): Result<AppInstallSummary, AppError>
function uninstall(AppUninstallContext $ctx): Result<AppInstallSummary, AppError>
```

## Updates

MVP should default to manual updates:

- The catalog marks newer reviewed versions as available.
- Store-admin shows capability changes and changelog before upgrade.
- The installer downloads the exact reviewed artifact and updates `deka.lock`.
- If hashes or review metadata do not match, install fails closed.

Automatic patch updates can come later for first-party apps only, guarded by
same-capability releases and exact artifact hashes.

## First-Party Apps

Good first-party candidates:

- Mail: transactional email provider wiring and templates.
- Filea: file storage and media management.
- Analytics: tenant-safe event aggregation dashboards.
- Search: storefront indexing and search UI.
- Reviews: product reviews, moderation, and storefront widgets.

Each first-party app should still ship as a normal reviewed app package. That
keeps the marketplace path honest and avoids privileged one-off runtime behavior.

## Implementation Phases

1. Manifest validation
   Add `type: "phpx-app"` and `app` metadata validation to package publish and
   install preflight. Require explicit capability declarations and reject unknown
   extension points.

2. Catalog API
   Add reviewed-app records backed by Linkhash releases. Expose discovery data
   for store-admin without exposing unreviewed package search results.

3. Tenant install records
   Store installed app metadata, approved capabilities, settings, and lifecycle
   status per tenant. Lock installs to immutable artifact hashes.

4. Installer path
   Add a platform install API used by store-admin. It checks Premium entitlement,
   downloads the artifact, validates the manifest, records approval, and updates
   tenant module state.

5. Runtime integration
   Resolve enabled app extension points for the tenant and call lifecycle or
   extension handlers with the app's approved capability set.

6. Store-admin UI
   Build catalog browsing, permission approval, settings, enable/disable,
   upgrade, and uninstall flows.

7. First-party apps
   Ship Mail first, then Filea. Use them as conformance fixtures for app
   capabilities, lifecycle hooks, and update policy.

## Definition Of Done

- Architecture distinguishes normal packages from apps without changing PHPX
  runtime semantics.
- Discovery, pricing, permissions, lifecycle, and update policy are specified.
- Implementation phases are ordered so Linkhash, store-admin, and runtime work
  can be split across agents.
