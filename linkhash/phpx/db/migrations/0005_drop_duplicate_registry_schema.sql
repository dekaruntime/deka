-- Retire linkha.sh's duplicate registry/VCS mirror (tana#901).
--
-- These tables mirrored data that git.tana.gg already owns. The web UI now
-- reads packages, versions, repos, issues and pull requests from the git
-- server's REST API (/api/packages, /api/scoped-packages/..., /api/repos/...),
-- so nothing in main.phpx queries them any more.
--
-- Safe to drop: the linkha.sh Postgres instance held 0 rows across all of these
-- (verified 2026-07-26); no data was ever loaded into this mirror.
--
-- What deliberately REMAINS in this schema: the lh_* identity tables
-- (lh_users, lh_sessions, lh_api_tokens, lh_orgs, lh_org_members,
-- lh_rate_limit_hits, lh_event_logs, lh_audit_logs) and runtime_probe_logs.
-- Those back linkha.sh's own OAuth login/session/PAT layer, which is not
-- git-server data and must move to the Tana IdP rather than to git.tana.gg.

-- Registry mirror (superseded by GET /api/packages + /api/scoped-packages/*)
DROP TABLE IF EXISTS "downloads";
DROP TABLE IF EXISTS "package_versions";
DROP TABLE IF EXISTS "package_releases";
DROP TABLE IF EXISTS "packages";

-- VCS mirror (superseded by /api/repos/:owner/:repo/{issues,pulls,labels})
DROP TABLE IF EXISTS "issue_labels";
DROP TABLE IF EXISTS "issue_comments";
DROP TABLE IF EXISTS "issue_sequences";
DROP TABLE IF EXISTS "issues";
DROP TABLE IF EXISTS "pull_comments";
DROP TABLE IF EXISTS "pull_sequences";
DROP TABLE IF EXISTS "pull_requests";
DROP TABLE IF EXISTS "labels";

-- Identity mirror of git-server accounts (superseded by /api/tokens and the
-- git server's own account records; linkha.sh keeps only its lh_* session layer)
DROP TABLE IF EXISTS "user_ssh_keys";
DROP TABLE IF EXISTS "user_tokens";
DROP TABLE IF EXISTS "users";
