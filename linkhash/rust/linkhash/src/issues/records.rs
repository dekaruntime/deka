use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Issue {
    pub id: i64,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: String,
    pub assignee: Option<String>,
    pub priority: Option<String>,
    pub repo: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct IssueComment {
    pub id: i64,
    pub issue_id: i64,
    pub body: String,
    pub author: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Label {
    pub id: i64,
    pub repo_owner: String,
    pub repo_name: String,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
    pub assignee: Option<String>,
    pub priority: Option<String>,
    pub repo: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<String>,
    pub assignee: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommentRequest {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct ListIssuesQuery {
    pub state: Option<String>,
    pub author: Option<String>,
    pub assignee: Option<String>,
    pub priority: Option<String>,
    pub repo: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct CommitRef {
    pub id: i64,
    pub issue_id: i64,
    pub commit_hash: String,
    pub repo: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommitRefRequest {
    pub commit_hash: String,
    pub repo: String,
}
