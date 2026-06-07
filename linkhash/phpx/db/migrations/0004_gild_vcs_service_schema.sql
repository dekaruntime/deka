-- gild-vcs service schema (owned by PHPX migrations)

CREATE TABLE IF NOT EXISTS "users" (
  "id" BIGSERIAL PRIMARY KEY,
  "username" TEXT NOT NULL UNIQUE,
  "email" TEXT,
  "password_hash" TEXT,
  "status" TEXT NOT NULL DEFAULT 'active',
  "email_verified_at" TIMESTAMPTZ,
  "display_name" TEXT,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "email" TEXT;
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "password_hash" TEXT;
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "status" TEXT NOT NULL DEFAULT 'active';
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "email_verified_at" TIMESTAMPTZ;
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "display_name" TEXT;
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE UNIQUE INDEX IF NOT EXISTS "idx_users_email_lower_unique"
  ON "users"(lower("email"))
  WHERE "email" IS NOT NULL;

CREATE TABLE IF NOT EXISTS "user_tokens" (
  "id" BIGSERIAL PRIMARY KEY,
  "user_id" BIGINT NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "token_name" TEXT NOT NULL,
  "token_hash" TEXT NOT NULL UNIQUE,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "last_used_at" TIMESTAMPTZ,
  "expires_at" TIMESTAMPTZ,
  "revoked_at" TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS "user_ssh_keys" (
  "id" BIGSERIAL PRIMARY KEY,
  "user_id" BIGINT NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "key_name" TEXT NOT NULL,
  "algorithm" TEXT NOT NULL,
  "public_key" TEXT NOT NULL,
  "fingerprint" TEXT NOT NULL UNIQUE,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "last_used_at" TIMESTAMPTZ,
  UNIQUE("user_id", "fingerprint")
);

CREATE TABLE IF NOT EXISTS "package_releases" (
  "id" BIGSERIAL PRIMARY KEY,
  "package_name" TEXT NOT NULL,
  "version" TEXT NOT NULL,
  "owner" TEXT NOT NULL,
  "repo" TEXT NOT NULL,
  "git_ref" TEXT NOT NULL DEFAULT 'HEAD',
  "description" TEXT,
  "manifest" JSONB,
  "api_snapshot" JSONB,
  "api_change_kind" TEXT,
  "required_bump" TEXT,
  "capability_metadata" JSONB,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE("package_name", "version")
);

ALTER TABLE "package_releases" ADD COLUMN IF NOT EXISTS "api_snapshot" JSONB;
ALTER TABLE "package_releases" ADD COLUMN IF NOT EXISTS "api_change_kind" TEXT;
ALTER TABLE "package_releases" ADD COLUMN IF NOT EXISTS "required_bump" TEXT;
ALTER TABLE "package_releases" ADD COLUMN IF NOT EXISTS "capability_metadata" JSONB;

CREATE INDEX IF NOT EXISTS "idx_package_releases_name"
  ON "package_releases"("package_name", "created_at" DESC);

CREATE TABLE IF NOT EXISTS "issues" (
  "id" SERIAL PRIMARY KEY,
  "repo_owner" TEXT NOT NULL,
  "repo_name" TEXT NOT NULL,
  "number" INT NOT NULL,
  "title" TEXT NOT NULL,
  "body" TEXT,
  "state" TEXT NOT NULL DEFAULT 'open',
  "author" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "updated_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "closed_at" TIMESTAMPTZ,
  UNIQUE("repo_owner", "repo_name", "number")
);

CREATE TABLE IF NOT EXISTS "issue_comments" (
  "id" SERIAL PRIMARY KEY,
  "issue_id" INT NOT NULL REFERENCES "issues"("id") ON DELETE CASCADE,
  "body" TEXT NOT NULL,
  "author" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "updated_at" TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS "labels" (
  "id" SERIAL PRIMARY KEY,
  "repo_owner" TEXT NOT NULL,
  "repo_name" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "color" TEXT NOT NULL DEFAULT '6e7681',
  "description" TEXT,
  UNIQUE("repo_owner", "repo_name", "name")
);

CREATE TABLE IF NOT EXISTS "issue_labels" (
  "issue_id" INT NOT NULL REFERENCES "issues"("id") ON DELETE CASCADE,
  "label_id" INT NOT NULL REFERENCES "labels"("id") ON DELETE CASCADE,
  PRIMARY KEY("issue_id", "label_id")
);

CREATE TABLE IF NOT EXISTS "issue_sequences" (
  "repo_owner" TEXT NOT NULL,
  "repo_name" TEXT NOT NULL,
  "next_number" INT NOT NULL DEFAULT 1,
  PRIMARY KEY("repo_owner", "repo_name")
);

CREATE TABLE IF NOT EXISTS "pull_requests" (
  "id" SERIAL PRIMARY KEY,
  "repo_owner" TEXT NOT NULL,
  "repo_name" TEXT NOT NULL,
  "number" INT NOT NULL,
  "title" TEXT NOT NULL,
  "body" TEXT,
  "state" TEXT NOT NULL DEFAULT 'open',
  "author" TEXT NOT NULL,
  "source_ref" TEXT NOT NULL,
  "target_ref" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "updated_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "closed_at" TIMESTAMPTZ,
  UNIQUE("repo_owner", "repo_name", "number")
);

CREATE TABLE IF NOT EXISTS "pull_comments" (
  "id" SERIAL PRIMARY KEY,
  "pull_id" INT NOT NULL REFERENCES "pull_requests"("id") ON DELETE CASCADE,
  "body" TEXT NOT NULL,
  "author" TEXT NOT NULL,
  "created_at" TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "updated_at" TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS "pull_sequences" (
  "repo_owner" TEXT NOT NULL,
  "repo_name" TEXT NOT NULL,
  "next_number" INT NOT NULL DEFAULT 1,
  PRIMARY KEY("repo_owner", "repo_name")
);

CREATE INDEX IF NOT EXISTS "idx_pulls_repo" ON "pull_requests"("repo_owner", "repo_name");
CREATE INDEX IF NOT EXISTS "idx_issues_repo" ON "issues"("repo_owner", "repo_name");
CREATE INDEX IF NOT EXISTS "idx_issues_state" ON "issues"("repo_owner", "repo_name", "state");
