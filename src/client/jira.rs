use serde_json::Value;
use tokio::sync::OnceCell;
use tracing::debug;

use crate::auth::SecretStore;
use crate::config::{AtlassianInstance, JiraFlavor};
use crate::error::Error;

use super::{
    HttpClient, RetryConfig, build_base_url, build_http_client, handle_response,
    handle_response_maybe_empty, read_sanitized_error_body,
};

pub struct JiraClient {
    http: HttpClient,
    /// A client without retry middleware, used for multipart/streaming
    /// requests that cannot be cloned for retries.
    no_retry_http: HttpClient,
    base_url: String,
    read_only: bool,
    /// Jira deployment flavor — Cloud has the v3 API (`/rest/api/3`),
    /// Data Center / Server only has v2. Routing decisions for search,
    /// bulk create, and archive/unarchive are made off this field.
    flavor: JiraFlavor,
    /// Lazily-resolved Jira Cloud `cloudId` for the automation API.
    /// Memoised once per process — automation routes hammer the same id.
    cloud_id: OnceCell<String>,
    /// Test-only override for the automation API host. When `Some`, the
    /// scheme+authority prefix replaces the hardcoded
    /// `https://api.atlassian.com` in URLs built by [`Self::automation_base_url`].
    /// Only ever set via the cfg-gated [`Self::with_automation_base_url`] —
    /// production code paths leave this as `None`.
    automation_base_override: Option<String>,
}

impl JiraClient {
    pub fn new(
        instance: &AtlassianInstance,
        profile: &str,
        store: &dyn SecretStore,
        cfg: RetryConfig,
    ) -> Result<Self, Error> {
        let http = build_http_client(instance, profile, "jira", store, cfg)?;
        // Build a separate client without retry middleware for multipart
        // requests. Multipart bodies are streaming and cannot be cloned,
        // which the retry middleware requires.
        let no_retry_http = if cfg.retries == 0 {
            // When retries is already 0 the main client has no retry layer,
            // so we can reuse it via a cheap clone (both are Arc-backed).
            http.clone()
        } else {
            build_http_client(instance, profile, "jira", store, RetryConfig::off())?
        };
        let base_url = build_base_url(instance, "/rest/api/2");
        Ok(Self {
            http,
            no_retry_http,
            base_url,
            read_only: instance.read_only,
            flavor: instance.resolved_flavor(),
            cloud_id: OnceCell::new(),
            automation_base_override: None,
        })
    }

    fn assert_writable(&self) -> Result<(), Error> {
        if self.read_only {
            return Err(Error::Config(
                "profile is read-only; write operations are blocked".into(),
            ));
        }
        Ok(())
    }

    fn agile_base_url(&self) -> String {
        self.base_url.replace("/rest/api/2", "/rest/agile/1.0")
    }

    fn v3_base_url(&self) -> String {
        self.base_url.replace("/rest/api/2", "/rest/api/3")
    }

    /// Public accessor for the agile API base URL.
    pub fn agile_url(&self) -> String {
        self.agile_base_url()
    }

    /// Public accessor for the REST API base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Search for issues matching a JQL query.
    ///
    /// On Cloud this hits the v3 `/search/jql` endpoint (the v2 `/search`
    /// route is being deprecated). On Data Center / Server — where v3 does
    /// not exist — this hits the v2 `/search` endpoint. Both return a
    /// `{startAt, maxResults, total, issues}` shape so downstream consumers
    /// are unaffected.
    pub async fn search_issues(
        &self,
        jql: &str,
        max_results: u32,
        fields: &[&str],
    ) -> Result<Value, Error> {
        let fields_str = fields.join(",");
        match self.flavor {
            JiraFlavor::Cloud => {
                let url = format!("{}/search/jql", self.v3_base_url());
                debug!("GET {url} jql={jql}");
                let resp = self
                    .http
                    .get(&url)
                    .query(&[
                        ("jql", jql),
                        ("maxResults", &max_results.to_string()),
                        ("fields", &fields_str),
                    ])
                    .send()
                    .await?;
                handle_response(resp).await
            }
            JiraFlavor::DataCenter => {
                let url = format!("{}/search", self.base_url);
                debug!("GET {url} jql={jql}");
                let resp = self
                    .http
                    .get(&url)
                    .query(&[
                        ("jql", jql),
                        ("maxResults", &max_results.to_string()),
                        ("fields", &fields_str),
                    ])
                    .send()
                    .await?;
                handle_response(resp).await
            }
        }
    }

    /// Search with auto-pagination: fetch all matching issues.
    ///
    /// On Cloud uses the v3 token-based pagination (`nextPageToken`). On
    /// Data Center uses the classic v2 `startAt` / `maxResults` pagination.
    /// In both cases the returned JSON is a synthetic
    /// `{startAt, maxResults, total, issues}` object whose `issues` array
    /// contains every matching issue, so downstream consumers see the same
    /// shape regardless of flavor.
    pub async fn search_issues_all(
        &self,
        jql: &str,
        page_size: u32,
        fields: &[&str],
    ) -> Result<Value, Error> {
        let fields_str = fields.join(",");
        let mut all_issues: Vec<Value> = Vec::new();

        match self.flavor {
            JiraFlavor::Cloud => {
                let url = format!("{}/search/jql", self.v3_base_url());
                let mut next_page_token: Option<String> = None;
                loop {
                    debug!(
                        "GET {url} jql={jql} maxResults={page_size} nextPageToken={next_page_token:?}"
                    );
                    let mut query: Vec<(&str, String)> = vec![
                        ("jql", jql.to_string()),
                        ("maxResults", page_size.to_string()),
                        ("fields", fields_str.clone()),
                    ];
                    if let Some(token) = &next_page_token {
                        query.push(("nextPageToken", token.clone()));
                    }
                    let resp = self.http.get(&url).query(&query).send().await?;
                    let page: Value = handle_response(resp).await?;
                    if let Some(issues) = page.get("issues").and_then(Value::as_array) {
                        if issues.is_empty() {
                            break;
                        }
                        all_issues.extend(issues.iter().cloned());
                    } else {
                        break;
                    }
                    // Token-based pagination: continue if nextPageToken is present and non-empty
                    match page.get("nextPageToken").and_then(Value::as_str) {
                        Some(token) if !token.is_empty() => {
                            next_page_token = Some(token.to_string());
                        }
                        _ => break,
                    }
                }
            }
            JiraFlavor::DataCenter => {
                let url = format!("{}/search", self.base_url);
                let mut start_at: u32 = 0;
                loop {
                    debug!("GET {url} jql={jql} startAt={start_at} maxResults={page_size}");
                    let query: Vec<(&str, String)> = vec![
                        ("jql", jql.to_string()),
                        ("startAt", start_at.to_string()),
                        ("maxResults", page_size.to_string()),
                        ("fields", fields_str.clone()),
                    ];
                    let resp = self.http.get(&url).query(&query).send().await?;
                    let page: Value = handle_response(resp).await?;
                    let total = page.get("total").and_then(Value::as_u64);
                    let Some(issues) = page.get("issues").and_then(Value::as_array) else {
                        break;
                    };
                    let returned = issues.len() as u32;
                    if returned == 0 {
                        break;
                    }
                    all_issues.extend(issues.iter().cloned());
                    start_at += returned;
                    match total {
                        Some(t) if u64::from(start_at) >= t => break,
                        None if returned < page_size => break,
                        _ => {}
                    }
                }
            }
        }

        Ok(serde_json::json!({
            "startAt": 0,
            "maxResults": all_issues.len(),
            "total": all_issues.len(),
            "issues": all_issues,
        }))
    }

    pub async fn get_issue(&self, issue_key: &str, fields: &[&str]) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}", self.base_url);
        let fields_str = if fields.is_empty() {
            None
        } else {
            Some(fields.join(","))
        };
        debug!("GET {url}");
        let mut req = self.http.get(&url);
        if let Some(f) = &fields_str {
            req = req.query(&[("fields", f.as_str())]);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    pub async fn create_issue(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/issue", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn bulk_create_issues(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        // The `{"issueUpdates": [...]}` payload shape is identical on v2 and
        // v3. On Cloud we prefer v3 (v2 `/issue/bulk` is being deprecated);
        // on Data Center we must use v2 because v3 does not exist there.
        let url = match self.flavor {
            JiraFlavor::Cloud => format!("{}/issue/bulk", self.v3_base_url()),
            JiraFlavor::DataCenter => format!("{}/issue/bulk", self.base_url),
        };
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_issue(&self, issue_key: &str, payload: &Value) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    pub async fn transition_issue(
        &self,
        issue_key: &str,
        transition_id: &str,
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/transitions", self.base_url);
        let payload = serde_json::json!({
            "transition": { "id": transition_id }
        });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    pub async fn get_transitions(&self, issue_key: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/transitions", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn assign_issue(&self, issue_key: &str, account_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/assignee", self.base_url);
        let payload = serde_json::json!({ "accountId": account_id });
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(&payload).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    pub async fn add_comment(&self, issue_key: &str, body: &str) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/comment", self.base_url);
        let payload = serde_json::json!({ "body": body });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn list_comments(&self, issue_key: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/comment", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_comment(&self, issue_key: &str, comment_id: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/comment/{comment_id}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_comment(&self, issue_key: &str, comment_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/comment/{comment_id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    // -- Projects --

    pub async fn get_projects(&self) -> Result<Value, Error> {
        let url = format!("{}/project", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_project(&self, key: &str) -> Result<Value, Error> {
        let url = format!("{}/project/{key}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn create_project(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/project", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_project(&self, key: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/project/{key}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_project(&self, key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/project/{key}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn get_project_statuses(&self, key: &str) -> Result<Value, Error> {
        let url = format!("{}/project/{key}/statuses", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_project_roles(&self, key: &str) -> Result<Value, Error> {
        let url = format!("{}/project/{key}/role", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn archive_project(&self, key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/project/{key}/archive", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn restore_project(&self, key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/project/{key}/restore", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Issue archive/unarchive (Jira Cloud Premium; v3 API) --

    /// Short-circuits with [`Error::Config`] when the instance is Data
    /// Center / Server. Issue archiving is a Jira Cloud Premium-only
    /// operation and the REST endpoints do not exist on self-hosted
    /// instances.
    fn assert_archive_supported(&self) -> Result<(), Error> {
        if self.flavor == JiraFlavor::DataCenter {
            return Err(Error::Config(
                "archive/unarchive is a Jira Cloud Premium operation; not available on Data Center"
                    .into(),
            ));
        }
        Ok(())
    }

    pub async fn archive_issue(&self, key: &str) -> Result<(), Error> {
        self.assert_archive_supported()?;
        self.assert_writable()?;
        // Jira Cloud has no single-issue archive endpoint; route through the
        // bulk endpoint with a one-element array. Same shape as
        // `archive_issues_bulk`, just with the response discarded.
        let url = format!("{}/issue/archive", self.v3_base_url());
        let payload = serde_json::json!({"issueIdsOrKeys": [key]});
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(&payload).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    pub async fn archive_issues_bulk(&self, keys: &[String]) -> Result<Value, Error> {
        self.assert_archive_supported()?;
        self.assert_writable()?;
        let url = format!("{}/issue/archive", self.v3_base_url());
        let payload = serde_json::json!({"issueIdsOrKeys": keys});
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn unarchive_issues_bulk(&self, keys: &[String]) -> Result<Value, Error> {
        self.assert_archive_supported()?;
        self.assert_writable()?;
        let url = format!("{}/issue/unarchive", self.v3_base_url());
        let payload = serde_json::json!({"issueIdsOrKeys": keys});
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn get_project_features(&self, key: &str) -> Result<Value, Error> {
        let url = format!("{}/project/{key}/features", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    // -- Boards --

    pub async fn get_boards(&self, project_key: Option<&str>) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board");
        debug!("GET {url}");
        let mut req = self.http.get(&url);
        if let Some(pk) = project_key {
            req = req.query(&[("projectKeyOrId", pk)]);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    pub async fn get_board(&self, board_id: u64) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board/{board_id}");
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_board_config(&self, board_id: u64) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board/{board_id}/configuration");
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_board_issues(
        &self,
        board_id: u64,
        max_results: u32,
        fields: &[&str],
    ) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board/{board_id}/issue");
        let fields_str = fields.join(",");
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("maxResults", &max_results.to_string()),
                ("fields", &fields_str),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn get_board_backlog(
        &self,
        board_id: u64,
        max_results: u32,
        fields: &[&str],
    ) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board/{board_id}/backlog");
        let fields_str = fields.join(",");
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("maxResults", &max_results.to_string()),
                ("fields", &fields_str),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn get_sprints(&self, board_id: u64, state: Option<&str>) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board/{board_id}/sprint");
        debug!("GET {url}");
        let mut req = self.http.get(&url);
        if let Some(s) = state {
            req = req.query(&[("state", s)]);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    pub async fn get_sprint_issues(
        &self,
        sprint_id: u64,
        max_results: u32,
        fields: &[&str],
    ) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/sprint/{sprint_id}/issue");
        let fields_str = fields.join(",");
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("maxResults", &max_results.to_string()),
                ("fields", &fields_str),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn get_sprint(&self, sprint_id: u64) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/sprint/{sprint_id}");
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn create_sprint(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/sprint");
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_sprint(&self, sprint_id: u64, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/sprint/{sprint_id}");
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_sprint(&self, sprint_id: u64) -> Result<(), Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/sprint/{sprint_id}");
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn move_issues_to_sprint(
        &self,
        sprint_id: u64,
        issue_keys: &[String],
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/sprint/{sprint_id}/issue");
        let payload = serde_json::json!({ "issues": issue_keys });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn move_issues_to_backlog(&self, issue_keys: &[String]) -> Result<(), Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/backlog/issue");
        let payload = serde_json::json!({ "issues": issue_keys });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn get_myself(&self) -> Result<Value, Error> {
        let url = format!("{}/myself", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_epics(&self, board_id: u64) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/board/{board_id}/epic");
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_epic(&self, epic_id_or_key: &str) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/epic/{epic_id_or_key}");
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_epic_issues(&self, epic_id_or_key: &str, limit: u32) -> Result<Value, Error> {
        let base = self.agile_base_url();
        let url = format!("{base}/epic/{epic_id_or_key}/issue?maxResults={limit}");
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn add_issues_to_epic(
        &self,
        epic_key: &str,
        issue_keys: &[String],
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/epic/{epic_key}/issue");
        let payload = serde_json::json!({ "issues": issue_keys });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response_maybe_empty(resp).await
    }

    pub async fn remove_issues_from_epic(&self, issue_keys: &[String]) -> Result<Value, Error> {
        self.assert_writable()?;
        let base = self.agile_base_url();
        let url = format!("{base}/epic/none/issue");
        let payload = serde_json::json!({ "issues": issue_keys });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response_maybe_empty(resp).await
    }

    pub async fn create_issue_link(
        &self,
        link_type: &str,
        inward_key: &str,
        outward_key: &str,
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issueLink", self.base_url);
        let payload = serde_json::json!({
            "type": { "name": link_type },
            "inwardIssue": { "key": inward_key },
            "outwardIssue": { "key": outward_key },
        });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn add_remote_link(
        &self,
        issue_key: &str,
        url: &str,
        title: &str,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let api_url = format!("{}/issue/{issue_key}/remotelink", self.base_url);
        let payload = serde_json::json!({
            "object": { "url": url, "title": title }
        });
        debug!("POST {api_url}");
        let resp = self.http.post(&api_url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn get_remote_links(&self, issue_key: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/remotelink", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_remote_link(&self, issue_key: &str, link_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/remotelink/{link_id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn get_issue_link_types(&self) -> Result<Value, Error> {
        let url = format!("{}/issueLinkType", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_issue(&self, issue_key: &str, delete_subtasks: bool) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}", self.base_url);
        debug!("DELETE {url} deleteSubtasks={delete_subtasks}");
        let mut req = self.http.delete(&url);
        if delete_subtasks {
            req = req.query(&[("deleteSubtasks", "true")]);
        }
        let resp = req.send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    // -- Worklog --

    pub async fn list_worklogs(&self, issue_key: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/worklog", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn add_worklog(
        &self,
        issue_key: &str,
        time_spent: &str,
        comment: Option<&str>,
        started: Option<&str>,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/worklog", self.base_url);
        let mut payload = serde_json::json!({ "timeSpent": time_spent });
        if let Some(c) = comment {
            payload["comment"] = Value::String(c.to_string());
        }
        if let Some(s) = started {
            payload["started"] = Value::String(s.to_string());
        }
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_worklog(&self, issue_key: &str, worklog_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/worklog/{worklog_id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Watchers --

    pub async fn get_watchers(&self, issue_key: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/watchers", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    // -- Notify --

    pub async fn notify_issue(&self, issue_key: &str, payload: &Value) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/notify", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Meta --

    pub async fn get_create_meta(
        &self,
        project: Option<&str>,
        issue_type: Option<&str>,
    ) -> Result<Value, Error> {
        let url = format!("{}/issue/createmeta", self.base_url);
        debug!("GET {url}");
        let mut req = self.http.get(&url);
        if let Some(p) = project {
            req = req.query(&[("projectKeys", p)]);
        }
        if let Some(it) = issue_type {
            req = req.query(&[("issuetypeNames", it)]);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    pub async fn get_edit_meta(&self, issue_key: &str) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/editmeta", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    // -- Filters --

    pub async fn list_favourite_filters(&self) -> Result<Value, Error> {
        let url = format!("{}/filter/favourite", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_filter(&self, id: &str) -> Result<Value, Error> {
        let url = format!("{}/filter/{id}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn create_filter(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/filter", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_filter(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/filter/{id}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_filter(&self, id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/filter/{id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Attachments --

    pub async fn attach_file(
        &self,
        issue_key: &str,
        file_path: &camino::Utf8Path,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/attachments", self.base_url);
        let file_name = file_path.file_name().unwrap_or("attachment").to_string();
        let file_bytes = std::fs::read(file_path.as_std_path())?;
        // Explicit Content-Type per part is required by strict multipart
        // parsers (including Prism's) and by some Atlassian endpoints.
        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(Error::Http)?;
        let form = reqwest::multipart::Form::new().part("file", part);
        debug!("POST {url} (multipart)");
        // Use the no-retry client: multipart bodies are streaming and cannot
        // be cloned, which the retry middleware requires for retries.
        let resp = self
            .no_retry_http
            .post(&url)
            .header("X-Atlassian-Token", "no-check")
            .multipart(form)
            .send()
            .await?;
        handle_response(resp).await
    }

    // -- Dashboards --

    pub async fn list_dashboards(&self) -> Result<Value, Error> {
        let url = format!("{}/dashboard", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_dashboard(&self, id: &str) -> Result<Value, Error> {
        let url = format!("{}/dashboard/{id}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn create_dashboard(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/dashboard", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_dashboard(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/dashboard/{id}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_dashboard(&self, id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/dashboard/{id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn copy_dashboard(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/dashboard/{id}/copy", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn list_dashboard_gadgets(&self, dashboard_id: &str) -> Result<Value, Error> {
        let url = format!("{}/dashboard/{dashboard_id}/gadget", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn add_dashboard_gadget(
        &self,
        dashboard_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/dashboard/{dashboard_id}/gadget", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_dashboard_gadget(
        &self,
        dashboard_id: &str,
        gadget_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/dashboard/{dashboard_id}/gadget/{gadget_id}",
            self.base_url
        );
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn remove_dashboard_gadget(
        &self,
        dashboard_id: &str,
        gadget_id: &str,
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/dashboard/{dashboard_id}/gadget/{gadget_id}",
            self.base_url
        );
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Fields --

    pub async fn get_fields(&self) -> Result<Value, Error> {
        let url = format!("{}/field", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    // -- Users --

    pub async fn search_users(&self, query: &str, max_results: u32) -> Result<Value, Error> {
        let url = format!("{}/user/search", self.base_url);
        debug!("GET {url} query={query}");
        let resp = self
            .http
            .get(&url)
            .query(&[("query", query), ("maxResults", &max_results.to_string())])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn get_user(&self, account_id: &str) -> Result<Value, Error> {
        let url = format!("{}/user", self.base_url);
        debug!("GET {url} accountId={account_id}");
        let resp = self
            .http
            .get(&url)
            .query(&[("accountId", account_id)])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn list_users(&self, max_results: u32) -> Result<Value, Error> {
        let url = format!("{}/users/search", self.base_url);
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[("maxResults", &max_results.to_string())])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn create_user(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/user", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_user(&self, account_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/user", self.base_url);
        debug!("DELETE {url} accountId={account_id}");
        let resp = self
            .http
            .delete(&url)
            .query(&[("accountId", account_id)])
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn get_assignable_users(
        &self,
        issue_key: &str,
        max_results: u32,
    ) -> Result<Value, Error> {
        let url = format!("{}/user/assignable/search", self.base_url);
        debug!("GET {url} issueKey={issue_key}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("issueKey", issue_key),
                ("maxResults", &max_results.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    // -- Groups --

    pub async fn list_groups(&self) -> Result<Value, Error> {
        let url = format!("{}/groups/picker", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_group(&self, name: &str) -> Result<Value, Error> {
        let url = format!("{}/group", self.base_url);
        debug!("GET {url} groupname={name}");
        let resp = self
            .http
            .get(&url)
            .query(&[("groupname", name)])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn create_group(&self, name: &str) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/group", self.base_url);
        let payload = serde_json::json!({ "name": name });
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_group(&self, name: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/group", self.base_url);
        debug!("DELETE {url} groupname={name}");
        let resp = self
            .http
            .delete(&url)
            .query(&[("groupname", name)])
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn get_group_members(&self, name: &str, max_results: u32) -> Result<Value, Error> {
        let url = format!("{}/group/member", self.base_url);
        debug!("GET {url} groupname={name}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("groupname", name),
                ("maxResults", &max_results.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn add_user_to_group(
        &self,
        group_name: &str,
        account_id: &str,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/group/user", self.base_url);
        let payload = serde_json::json!({ "accountId": account_id });
        debug!("POST {url} groupname={group_name}");
        let resp = self
            .http
            .post(&url)
            .query(&[("groupname", group_name)])
            .json(&payload)
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn remove_user_from_group(
        &self,
        group_name: &str,
        account_id: &str,
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/group/user", self.base_url);
        debug!("DELETE {url} groupname={group_name} accountId={account_id}");
        let resp = self
            .http
            .delete(&url)
            .query(&[("groupname", group_name), ("accountId", account_id)])
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Versions --

    pub async fn get_project_versions(&self, project_key: &str) -> Result<Value, Error> {
        let url = format!("{}/project/{project_key}/versions", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_version(&self, id: &str) -> Result<Value, Error> {
        let url = format!("{}/version/{id}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn update_version(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/version/{id}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn create_version(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/version", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_version(&self, version_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/version/{version_id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn release_version(
        &self,
        version_id: &str,
        release_date: &str,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/version/{version_id}", self.base_url);
        let payload = serde_json::json!({
            "released": true,
            "releaseDate": release_date,
        });
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    // -- Components --

    pub async fn get_project_components(&self, project_key: &str) -> Result<Value, Error> {
        let url = format!("{}/project/{project_key}/components", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn get_component(&self, id: &str) -> Result<Value, Error> {
        let url = format!("{}/component/{id}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    pub async fn create_component(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/component", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn update_component(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/component/{id}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_component(&self, component_id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/component/{component_id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Vote --

    pub async fn vote_issue(&self, issue_key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/votes", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn unvote_issue(&self, issue_key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/votes", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Changelog --

    pub async fn get_changelog(
        &self,
        issue_key: &str,
        max_results: u32,
        start_at: u32,
    ) -> Result<Value, Error> {
        let url = format!("{}/issue/{issue_key}/changelog", self.base_url);
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("maxResults", &max_results.to_string()),
                ("startAt", &start_at.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    /// Generic offset-based auto-pagination for Jira list endpoints.
    /// `items_key` is the JSON key containing the array of items (e.g. "values", "issues").
    pub async fn paginate_offset(
        &self,
        url: &str,
        page_size: u32,
        items_key: &str,
        extra_query: &[(&str, &str)],
    ) -> Result<Value, Error> {
        let mut all_items = Vec::new();
        let mut start_at = 0u32;
        loop {
            debug!("GET {url} startAt={start_at} maxResults={page_size}");
            let page_str = page_size.to_string();
            let start_str = start_at.to_string();
            let mut query: Vec<(&str, &str)> =
                vec![("startAt", &start_str), ("maxResults", &page_str)];
            query.extend_from_slice(extra_query);
            let resp = self.http.get(url).query(&query).send().await?;
            let page: Value = handle_response(resp).await?;
            let total = page.get("total").and_then(Value::as_u64);
            let page_len = if let Some(items) = page.get(items_key).and_then(Value::as_array) {
                if items.is_empty() {
                    break;
                }
                let len = items.len() as u32;
                all_items.extend(items.iter().cloned());
                start_at += len;
                len
            } else {
                break;
            };
            // Use total if available, otherwise check if we got a partial page
            if let Some(t) = total {
                if start_at as u64 >= t {
                    break;
                }
            } else if page_len < page_size {
                break;
            }
        }
        Ok(serde_json::json!({
            items_key: all_items,
            "total": all_items.len(),
        }))
    }

    // -- Watch --

    pub async fn watch_issue(&self, issue_key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/issue/{issue_key}/watchers", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    pub async fn unwatch_issue(&self, issue_key: &str) -> Result<(), Error> {
        self.assert_writable()?;
        // Need current user's accountId
        let me = self.get_myself().await?;
        let account_id = me["accountId"].as_str().ok_or_else(|| Error::Api {
            status: 0,
            message: "cannot determine current user accountId".into(),
        })?;
        let url = format!("{}/issue/{issue_key}/watchers", self.base_url);
        debug!("DELETE {url} accountId={account_id}");
        let resp = self
            .http
            .delete(&url)
            .query(&[("accountId", account_id)])
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Admin: Generic REST helpers --

    async fn admin_get(&self, path: &str) -> Result<Value, Error> {
        let url = format!("{}{path}", self.base_url);
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    async fn admin_post(&self, path: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}{path}", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    async fn admin_put(&self, path: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}{path}", self.base_url);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    async fn admin_delete(&self, path: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}{path}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }

    // -- Admin: Issue Types --

    pub async fn list_issue_types(&self) -> Result<Value, Error> {
        self.admin_get("/issuetype").await
    }

    pub async fn get_issue_type(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/issuetype/{id}")).await
    }

    pub async fn create_issue_type(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/issuetype", payload).await
    }

    pub async fn update_issue_type(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.admin_put(&format!("/issuetype/{id}"), payload).await
    }

    pub async fn delete_issue_type(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/issuetype/{id}")).await
    }

    // -- Admin: Priorities --

    pub async fn list_priorities(&self) -> Result<Value, Error> {
        self.admin_get("/priority").await
    }

    pub async fn get_priority(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/priority/{id}")).await
    }

    pub async fn create_priority(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/priority", payload).await
    }

    pub async fn update_priority(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.admin_put(&format!("/priority/{id}"), payload).await
    }

    pub async fn delete_priority(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/priority/{id}")).await
    }

    // -- Admin: Resolutions --

    pub async fn list_resolutions(&self) -> Result<Value, Error> {
        self.admin_get("/resolution").await
    }

    pub async fn get_resolution(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/resolution/{id}")).await
    }

    pub async fn create_resolution(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/resolution", payload).await
    }

    pub async fn update_resolution(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.admin_put(&format!("/resolution/{id}"), payload).await
    }

    pub async fn delete_resolution(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/resolution/{id}")).await
    }

    // -- Admin: Statuses --

    pub async fn list_statuses(&self) -> Result<Value, Error> {
        self.admin_get("/status").await
    }

    pub async fn get_status(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/status/{id}")).await
    }

    pub async fn list_status_categories(&self) -> Result<Value, Error> {
        self.admin_get("/statuscategory").await
    }

    // -- Admin: Screens --

    pub async fn list_screens(&self) -> Result<Value, Error> {
        self.admin_get("/screens").await
    }

    pub async fn get_screen(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/screens/{id}")).await
    }

    pub async fn get_screen_tabs(&self, screen_id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/screens/{screen_id}/tabs")).await
    }

    pub async fn create_screen(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/screens", payload).await
    }

    pub async fn delete_screen(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/screens/{id}")).await
    }

    pub async fn get_screen_tab_fields(
        &self,
        screen_id: &str,
        tab_id: &str,
    ) -> Result<Value, Error> {
        self.admin_get(&format!("/screens/{screen_id}/tabs/{tab_id}/fields"))
            .await
    }

    // -- Admin: Workflows --

    pub async fn list_workflows(&self) -> Result<Value, Error> {
        self.admin_get("/workflow").await
    }

    pub async fn get_workflow(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/workflow/{id}")).await
    }

    // -- Admin: Workflow Schemes --

    pub async fn list_workflow_schemes(&self) -> Result<Value, Error> {
        self.admin_get("/workflowscheme").await
    }

    pub async fn get_workflow_scheme(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/workflowscheme/{id}")).await
    }

    pub async fn create_workflow_scheme(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/workflowscheme", payload).await
    }

    pub async fn update_workflow_scheme(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.admin_put(&format!("/workflowscheme/{id}"), payload)
            .await
    }

    pub async fn delete_workflow_scheme(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/workflowscheme/{id}")).await
    }

    // -- Admin: Permission Schemes --

    pub async fn list_permission_schemes(&self) -> Result<Value, Error> {
        self.admin_get("/permissionscheme").await
    }

    pub async fn get_permission_scheme(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/permissionscheme/{id}")).await
    }

    pub async fn create_permission_scheme(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/permissionscheme", payload).await
    }

    pub async fn update_permission_scheme(
        &self,
        id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_put(&format!("/permissionscheme/{id}"), payload)
            .await
    }

    pub async fn delete_permission_scheme(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/permissionscheme/{id}")).await
    }

    // -- Admin: Notification Schemes --

    pub async fn list_notification_schemes(&self) -> Result<Value, Error> {
        self.admin_get("/notificationscheme").await
    }

    pub async fn get_notification_scheme(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/notificationscheme/{id}")).await
    }

    pub async fn create_notification_scheme(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/notificationscheme", payload).await
    }

    pub async fn update_notification_scheme(
        &self,
        id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_put(&format!("/notificationscheme/{id}"), payload)
            .await
    }

    pub async fn delete_notification_scheme(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/notificationscheme/{id}"))
            .await
    }

    // -- Admin: Issue Security Schemes --

    pub async fn list_issue_security_schemes(&self) -> Result<Value, Error> {
        self.admin_get("/issuesecurityschemes").await
    }

    pub async fn get_issue_security_scheme(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/issuesecurityschemes/{id}")).await
    }

    pub async fn create_issue_security_scheme(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/issuesecurityschemes", payload).await
    }

    pub async fn update_issue_security_scheme(
        &self,
        id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_put(&format!("/issuesecurityschemes/{id}"), payload)
            .await
    }

    pub async fn delete_issue_security_scheme(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/issuesecurityschemes/{id}"))
            .await
    }

    // -- Admin: Field Configurations --

    pub async fn list_field_configurations(&self) -> Result<Value, Error> {
        self.admin_get("/fieldconfiguration").await
    }

    pub async fn get_field_configuration(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/fieldconfiguration/{id}")).await
    }

    pub async fn create_field_configuration(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/fieldconfiguration", self.base_url);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    pub async fn delete_field_configuration(&self, id: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/fieldconfiguration/{id}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    // -- Admin: Project Categories --

    pub async fn list_project_categories(&self) -> Result<Value, Error> {
        self.admin_get("/projectCategory").await
    }

    pub async fn get_project_category(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/projectCategory/{id}")).await
    }

    pub async fn create_project_category(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/projectCategory", payload).await
    }

    pub async fn update_project_category(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.admin_put(&format!("/projectCategory/{id}"), payload)
            .await
    }

    pub async fn delete_project_category(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/projectCategory/{id}")).await
    }

    // -- Admin: Server Info --

    pub async fn get_server_info(&self) -> Result<Value, Error> {
        self.admin_get("/serverInfo").await
    }

    // -- Admin: Webhooks --

    pub async fn list_webhooks(&self) -> Result<Value, Error> {
        self.admin_get("/webhook").await
    }

    pub async fn get_webhook(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/webhook/{id}")).await
    }

    pub async fn create_webhook(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/webhook", payload).await
    }

    pub async fn delete_webhook(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/webhook/{id}")).await
    }

    // -- Admin: Audit Records --

    pub async fn get_audit_records(
        &self,
        limit: u32,
        offset: u32,
        filter: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Value, Error> {
        let url = format!("{}/auditing/record", self.base_url);
        debug!("GET {url}");
        let mut req = self.http.get(&url).query(&[
            ("maxResults", &limit.to_string()),
            ("offset", &offset.to_string()),
        ]);
        if let Some(f) = filter {
            req = req.query(&[("filter", f)]);
        }
        if let Some(f) = from {
            req = req.query(&[("from", f)]);
        }
        if let Some(t) = to {
            req = req.query(&[("to", t)]);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    // -- Admin: Permissions --

    pub async fn get_all_permissions(&self) -> Result<Value, Error> {
        self.admin_get("/permissions").await
    }

    pub async fn get_my_permissions(&self) -> Result<Value, Error> {
        self.admin_get("/mypermissions").await
    }

    // -- Admin: Issue Link Types --

    pub async fn get_issue_link_type(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/issueLinkType/{id}")).await
    }

    pub async fn create_issue_link_type(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/issueLinkType", payload).await
    }

    pub async fn update_issue_link_type(&self, id: &str, payload: &Value) -> Result<Value, Error> {
        self.admin_put(&format!("/issueLinkType/{id}"), payload)
            .await
    }

    pub async fn delete_issue_link_type(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/issueLinkType/{id}")).await
    }

    // -- Admin: Issue Links --

    pub async fn get_issue_link(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/issueLink/{id}")).await
    }

    pub async fn delete_issue_link(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/issueLink/{id}")).await
    }

    // -- Admin: Fields (Custom) --

    pub async fn create_field(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/field", payload).await
    }

    pub async fn delete_field(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/field/{id}")).await
    }

    pub async fn trash_field(&self, id: &str) -> Result<Value, Error> {
        self.admin_post(&format!("/field/{id}/trash"), &serde_json::json!({}))
            .await
    }

    pub async fn restore_field(&self, id: &str) -> Result<Value, Error> {
        self.admin_post(&format!("/field/{id}/restore"), &serde_json::json!({}))
            .await
    }

    // -- Admin: Field Contexts --

    pub async fn field_contexts_list(
        &self,
        field_id: &str,
        max_results: u32,
        start_at: u32,
    ) -> Result<Value, Error> {
        let url = format!("{}/field/{field_id}/context", self.base_url);
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("maxResults", &max_results.to_string()),
                ("startAt", &start_at.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn field_contexts_list_all(&self, field_id: &str) -> Result<Value, Error> {
        let url = format!("{}/field/{field_id}/context", self.base_url);
        self.paginate_offset(&url, 100, "values", &[]).await
    }

    pub async fn create_field_context(
        &self,
        field_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_post(&format!("/field/{field_id}/context"), payload)
            .await
    }

    pub async fn update_field_context(
        &self,
        field_id: &str,
        context_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_put(&format!("/field/{field_id}/context/{context_id}"), payload)
            .await
    }

    pub async fn delete_field_context(
        &self,
        field_id: &str,
        context_id: &str,
    ) -> Result<(), Error> {
        self.admin_delete(&format!("/field/{field_id}/context/{context_id}"))
            .await
    }

    pub async fn field_context_project_mappings(
        &self,
        field_id: &str,
        context_id: &str,
        max_results: u32,
        start_at: u32,
    ) -> Result<Value, Error> {
        let url = format!("{}/field/{field_id}/context/projectmapping", self.base_url);
        debug!("GET {url} contextId={context_id}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("startAt", &start_at.to_string()),
                ("maxResults", &max_results.to_string()),
                ("contextId", &context_id.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn field_context_project_mappings_all(
        &self,
        field_id: &str,
        context_id: &str,
    ) -> Result<Value, Error> {
        let url = format!("{}/field/{field_id}/context/projectmapping", self.base_url);
        self.paginate_offset(&url, 50, "values", &[("contextId", context_id)])
            .await
    }

    pub async fn field_context_assign_projects(
        &self,
        field_id: &str,
        context_id: &str,
        project_ids: &[String],
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/project",
            self.base_url
        );
        debug!("PUT {url}");
        let resp = self
            .http
            .put(&url)
            .json(&serde_json::json!({ "projectIds": project_ids }))
            .send()
            .await?;
        handle_response_maybe_empty(resp).await
    }

    pub async fn field_context_remove_projects(
        &self,
        field_id: &str,
        context_id: &str,
        project_ids: &[String],
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/project/remove",
            self.base_url
        );
        debug!("POST {url}");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "projectIds": project_ids }))
            .send()
            .await?;
        handle_response_maybe_empty(resp).await
    }

    pub async fn field_context_issue_type_mappings(
        &self,
        field_id: &str,
        context_id: &str,
        max_results: u32,
        start_at: u32,
    ) -> Result<Value, Error> {
        let url = format!(
            "{}/field/{field_id}/context/issuetypemapping",
            self.base_url
        );
        debug!("GET {url} contextId={context_id}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("startAt", &start_at.to_string()),
                ("maxResults", &max_results.to_string()),
                ("contextId", &context_id.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn field_context_issue_type_mappings_all(
        &self,
        field_id: &str,
        context_id: &str,
    ) -> Result<Value, Error> {
        let url = format!(
            "{}/field/{field_id}/context/issuetypemapping",
            self.base_url
        );
        self.paginate_offset(&url, 50, "values", &[("contextId", context_id)])
            .await
    }

    pub async fn field_context_assign_issue_types(
        &self,
        field_id: &str,
        context_id: &str,
        issue_type_ids: &[String],
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/issuetype",
            self.base_url
        );
        debug!("PUT {url}");
        let resp = self
            .http
            .put(&url)
            .json(&serde_json::json!({ "issueTypeIds": issue_type_ids }))
            .send()
            .await?;
        handle_response_maybe_empty(resp).await
    }

    pub async fn field_context_remove_issue_types(
        &self,
        field_id: &str,
        context_id: &str,
        issue_type_ids: &[String],
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/issuetype/remove",
            self.base_url
        );
        debug!("POST {url}");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "issueTypeIds": issue_type_ids }))
            .send()
            .await?;
        handle_response_maybe_empty(resp).await
    }

    // -- Admin: Field Context Options --

    pub async fn field_options_list(
        &self,
        field_id: &str,
        context_id: &str,
        max_results: u32,
        start_at: u32,
    ) -> Result<Value, Error> {
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/option",
            self.base_url
        );
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("maxResults", &max_results.to_string()),
                ("startAt", &start_at.to_string()),
            ])
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn field_options_list_all(
        &self,
        field_id: &str,
        context_id: &str,
    ) -> Result<Value, Error> {
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/option",
            self.base_url
        );
        self.paginate_offset(&url, 100, "values", &[]).await
    }

    pub async fn field_options_create(
        &self,
        field_id: &str,
        context_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_post(
            &format!("/field/{field_id}/context/{context_id}/option"),
            payload,
        )
        .await
    }

    pub async fn field_options_update(
        &self,
        field_id: &str,
        context_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_put(
            &format!("/field/{field_id}/context/{context_id}/option"),
            payload,
        )
        .await
    }

    pub async fn field_option_delete(
        &self,
        field_id: &str,
        context_id: &str,
        option_id: &str,
    ) -> Result<(), Error> {
        self.admin_delete(&format!(
            "/field/{field_id}/context/{context_id}/option/{option_id}"
        ))
        .await
    }

    pub async fn field_options_reorder(
        &self,
        field_id: &str,
        context_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!(
            "{}/field/{field_id}/context/{context_id}/option/move",
            self.base_url
        );
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response_maybe_empty(resp).await
    }

    // -- Admin: Standalone Roles --

    pub async fn list_roles(&self) -> Result<Value, Error> {
        self.admin_get("/role").await
    }

    pub async fn get_role(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/role/{id}")).await
    }

    pub async fn create_role(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/role", payload).await
    }

    pub async fn delete_role(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/role/{id}")).await
    }

    // -- Admin: Issue Type Schemes --

    pub async fn list_issue_type_schemes(&self) -> Result<Value, Error> {
        self.admin_get("/issuetypescheme").await
    }

    pub async fn get_issue_type_scheme(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/issuetypescheme/{id}")).await
    }

    pub async fn create_issue_type_scheme(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_post("/issuetypescheme", payload).await
    }

    pub async fn update_issue_type_scheme(
        &self,
        id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.admin_put(&format!("/issuetypescheme/{id}"), payload)
            .await
    }

    pub async fn delete_issue_type_scheme(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/issuetypescheme/{id}")).await
    }

    // -- Admin: Announcement Banner --

    pub async fn get_banner(&self) -> Result<Value, Error> {
        self.admin_get("/announcementBanner").await
    }

    pub async fn set_banner(&self, payload: &Value) -> Result<Value, Error> {
        self.admin_put("/announcementBanner", payload).await
    }

    // -- Admin: Configuration --

    pub async fn get_configuration(&self) -> Result<Value, Error> {
        self.admin_get("/configuration").await
    }

    // -- Admin: Async Tasks --

    pub async fn get_task(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/task/{id}")).await
    }

    pub async fn cancel_task(&self, id: &str) -> Result<Value, Error> {
        self.admin_post(&format!("/task/{id}/cancel"), &serde_json::json!({}))
            .await
    }

    // -- Admin: Attachment Admin --

    pub async fn get_attachment(&self, id: &str) -> Result<Value, Error> {
        self.admin_get(&format!("/attachment/{id}")).await
    }

    pub async fn delete_attachment(&self, id: &str) -> Result<(), Error> {
        self.admin_delete(&format!("/attachment/{id}")).await
    }

    pub async fn get_attachment_meta(&self) -> Result<Value, Error> {
        self.admin_get("/attachment/meta").await
    }

    // -- Groups: Search --

    pub async fn search_groups(&self, query: &str, max_results: u32) -> Result<Value, Error> {
        let url = format!("{}/groups/picker", self.base_url);
        debug!("GET {url} query={query}");
        let resp = self
            .http
            .get(&url)
            .query(&[("query", query), ("maxResults", &max_results.to_string())])
            .send()
            .await?;
        handle_response(resp).await
    }

    // -- Filters: Search/List --

    pub async fn search_filters(
        &self,
        name: Option<&str>,
        favourites: bool,
        mine: bool,
    ) -> Result<Value, Error> {
        if favourites {
            return self.list_favourite_filters().await;
        }
        let url = if mine {
            format!("{}/filter/my", self.base_url)
        } else {
            format!("{}/filter/search", self.base_url)
        };
        debug!("GET {url}");
        let mut req = self.http.get(&url);
        if let Some(n) = name {
            req = req.query(&[("filterName", n)]);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    // -- Admin: Labels --

    pub async fn list_labels(&self, max_results: u32) -> Result<Value, Error> {
        let url = format!("{}/label", self.base_url);
        debug!("GET {url}");
        let resp = self
            .http
            .get(&url)
            .query(&[("maxResults", &max_results.to_string())])
            .send()
            .await?;
        handle_response(resp).await
    }

    // -- Fields: list (alias used by `atl jira issue check`) --

    /// List all fields visible to the user.
    ///
    /// On Cloud this hits the v3 `/field` endpoint; on Data Center / Server it
    /// falls back to the v2 endpoint (which exists on both flavors). The
    /// returned `Value` is the raw array Jira gives us — the caller is
    /// expected to extract `id` / `name` per element.
    pub async fn list_fields(&self) -> Result<Value, Error> {
        match self.flavor {
            JiraFlavor::Cloud => {
                let url = format!("{}/field", self.v3_base_url());
                debug!("GET {url}");
                let resp = self.http.get(&url).send().await?;
                handle_response(resp).await
            }
            JiraFlavor::DataCenter => self.get_fields().await,
        }
    }

    // -- Automation rules (Jira Cloud only) --

    /// Returns the site root URL (no API path), used for the cloud-id
    /// auto-fetch. Strips the `/rest/api/2` suffix that `base_url` carries.
    fn site_base_url(&self) -> String {
        self.base_url
            .strip_suffix("/rest/api/2")
            .map(str::to_owned)
            .unwrap_or_else(|| self.base_url.clone())
    }

    /// Resolve the Jira Cloud `cloudId` for this instance, hitting
    /// `<site>/_edge/tenant_info` on first call and caching the result for the
    /// lifetime of the client.
    ///
    /// Returns [`Error::Config`] for non-Cloud sites (the `_edge/tenant_info`
    /// endpoint only exists on Atlassian Cloud).
    pub async fn get_cloud_id(&self) -> Result<String, Error> {
        if self.flavor != JiraFlavor::Cloud {
            return Err(Error::Config(
                "automation API requires a Jira Cloud instance; this site does not expose tenant_info".into(),
            ));
        }
        self.cloud_id
            .get_or_try_init(|| async {
                let url = format!("{}/_edge/tenant_info", self.site_base_url());
                debug!("GET {url}");
                let resp = self.http.get(&url).send().await?;
                let status = resp.status();
                if !status.is_success() {
                    if status == reqwest::StatusCode::NOT_FOUND {
                        return Err(Error::Config(
                            "automation API requires a Jira Cloud instance; this site does not expose tenant_info".into(),
                        ));
                    }
                    let body = read_sanitized_error_body(resp).await;
                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        return Err(Error::Auth(format!("{}: {body}", status.as_u16())));
                    }
                    return Err(Error::Api {
                        status: status.as_u16(),
                        message: body,
                    });
                }
                let body: Value = resp.json().await?;
                body.get("cloudId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        Error::InvalidResponse(
                            "tenant_info response missing cloudId field".into(),
                        )
                    })
            })
            .await
            .cloned()
    }

    /// Build the automation API base URL.
    ///
    /// Production: `https://api.atlassian.com/automation/public/jira/{cloud_id}`.
    ///
    /// If [`Self::automation_base_override`] is `Some(host)` (test-only seam set
    /// via [`Self::with_automation_base_url`]), the host replaces
    /// `https://api.atlassian.com` and the `/automation/public/jira/{cloud_id}`
    /// suffix is appended verbatim. The override is treated as a
    /// **scheme + authority** prefix only (e.g. `http://127.0.0.1:38421`); it
    /// must not include the `/automation/...` path — this method always appends
    /// it.
    fn automation_base_url(&self, cloud_id: &str) -> String {
        match &self.automation_base_override {
            Some(host) => format!("{host}/automation/public/jira/{cloud_id}"),
            None => format!("https://api.atlassian.com/automation/public/jira/{cloud_id}"),
        }
    }

    /// List automation rules. `cursor` and `limit` are forwarded as query params.
    pub async fn list_automation_rules(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Value, Error> {
        let cloud_id = self.get_cloud_id().await?;
        let url = format!(
            "{}/rest/v1/rule/summary",
            self.automation_base_url(&cloud_id)
        );
        debug!("GET {url}");
        let mut req = self.http.get(&url);
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(c) = cursor {
            query.push(("cursor".into(), c.into()));
        }
        if let Some(n) = limit {
            query.push(("limit".into(), n.to_string()));
        }
        if !query.is_empty() {
            req = req.query(&query);
        }
        let resp = req.send().await?;
        handle_response(resp).await
    }

    /// Get the full definition of an automation rule by UUID.
    pub async fn get_automation_rule(&self, uuid: &str) -> Result<Value, Error> {
        let cloud_id = self.get_cloud_id().await?;
        let url = format!(
            "{}/rest/v1/rule/{uuid}",
            self.automation_base_url(&cloud_id)
        );
        debug!("GET {url}");
        let resp = self.http.get(&url).send().await?;
        handle_response(resp).await
    }

    /// Create a new automation rule from a JSON payload.
    pub async fn create_automation_rule(&self, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let cloud_id = self.get_cloud_id().await?;
        let url = format!("{}/rest/v1/rule", self.automation_base_url(&cloud_id));
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    /// Replace an existing automation rule with the supplied JSON payload.
    pub async fn update_automation_rule(
        &self,
        uuid: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let cloud_id = self.get_cloud_id().await?;
        let url = format!(
            "{}/rest/v1/rule/{uuid}",
            self.automation_base_url(&cloud_id)
        );
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response_maybe_empty(resp).await
    }

    /// Build the JSON body for `set_automation_rule_state`.
    fn automation_state_payload(enabled: bool) -> Value {
        serde_json::json!({ "state": if enabled { "ENABLED" } else { "DISABLED" } })
    }

    /// Enable or disable an automation rule.
    pub async fn set_automation_rule_state(
        &self,
        uuid: &str,
        enabled: bool,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let cloud_id = self.get_cloud_id().await?;
        let url = format!(
            "{}/rest/v1/rule/{uuid}/state",
            self.automation_base_url(&cloud_id)
        );
        let payload = Self::automation_state_payload(enabled);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(&payload).send().await?;
        handle_response_maybe_empty(resp).await
    }

    /// Delete an automation rule. Per Atlassian, the rule must be disabled
    /// first; the upstream 4xx response is surfaced unmodified if it isn't.
    pub async fn delete_automation_rule(&self, uuid: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let cloud_id = self.get_cloud_id().await?;
        let url = format!(
            "{}/rest/v1/rule/{uuid}",
            self.automation_base_url(&cloud_id)
        );
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = read_sanitized_error_body(resp).await;
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(Error::Auth(format!("{}: {body}", status.as_u16())));
            }
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(Error::NotFound(body));
            }
            Err(Error::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

/// Test-only seams.
///
/// These constructors are exposed only when the crate is compiled for tests
/// or with the `test-util` feature enabled. They let tests redirect HTTP
/// calls to a local mock server without leaking knobs into the production
/// API surface.
#[cfg(any(test, feature = "test-util"))]
impl JiraClient {
    /// Override the automation API host for testing.
    ///
    /// `host` must be a **scheme + authority** prefix only — e.g.
    /// `http://127.0.0.1:38421` or the value of [`httpmock::MockServer::url`]
    /// for the empty path. The `/automation/public/jira/{cloud_id}` suffix is
    /// appended by [`Self::automation_base_url`] and must not be included in
    /// `host`.
    ///
    /// Production code never sets this.
    #[must_use]
    pub fn with_automation_base_url(mut self, host: impl Into<String>) -> Self {
        self.automation_base_override = Some(host.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::InMemoryStore;
    use crate::config::AuthType;
    use crate::test_util::env_lock;

    /// Builds a [`JiraClient`] configured for Data Center with an in-memory
    /// secret store so construction succeeds without a real keyring. The
    /// archive short-circuit runs before any HTTP call, so no network is
    /// involved.
    fn make_dc_client() -> JiraClient {
        let _g = env_lock();
        // SAFETY: env access is serialized by env_lock() for the whole test.
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = AtlassianInstance {
            domain: "jira.company.com".to_string(),
            email: Some("alice@company.com".into()),
            api_token: Some("irrelevant".into()),
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: Some(JiraFlavor::DataCenter),
        };
        let store = InMemoryStore::new();
        JiraClient::new(&inst, "default", &store, RetryConfig::off())
            .expect("JiraClient should build")
    }

    #[tokio::test]
    async fn archive_issue_short_circuits_on_data_center() {
        let client = make_dc_client();
        match client.archive_issue("PROJ-1").await {
            Err(Error::Config(msg)) => {
                assert!(
                    msg.contains("Data Center"),
                    "expected DC message, got: {msg}"
                );
            }
            other => panic!("expected Error::Config on DC, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn archive_issues_bulk_short_circuits_on_data_center() {
        let client = make_dc_client();
        let keys = vec!["PROJ-1".to_string(), "PROJ-2".to_string()];
        match client.archive_issues_bulk(&keys).await {
            Err(Error::Config(msg)) => {
                assert!(
                    msg.contains("Data Center"),
                    "expected DC message, got: {msg}"
                );
            }
            other => panic!("expected Error::Config on DC, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn unarchive_issues_bulk_short_circuits_on_data_center() {
        let client = make_dc_client();
        let keys = vec!["PROJ-1".to_string()];
        match client.unarchive_issues_bulk(&keys).await {
            Err(Error::Config(msg)) => {
                assert!(
                    msg.contains("Data Center"),
                    "expected DC message, got: {msg}"
                );
            }
            other => panic!("expected Error::Config on DC, got: {other:?}"),
        }
    }

    // -- Automation helpers (pure) --

    #[test]
    fn automation_state_payload_enabled() {
        let v = JiraClient::automation_state_payload(true);
        assert_eq!(v, serde_json::json!({"state": "ENABLED"}));
    }

    #[test]
    fn automation_state_payload_disabled() {
        let v = JiraClient::automation_state_payload(false);
        assert_eq!(v, serde_json::json!({"state": "DISABLED"}));
    }

    #[test]
    fn automation_base_url_format() {
        let client = make_dc_client();
        let url = client.automation_base_url("abc-123");
        assert_eq!(
            url,
            "https://api.atlassian.com/automation/public/jira/abc-123"
        );
    }

    #[test]
    fn automation_base_url_passes_uuid_through_verbatim() {
        // Real cloudIds are UUIDs containing hyphens. The format string must
        // not URL-encode them or normalize the case.
        let client = make_dc_client();
        let uuid = "11111111-2222-3333-4444-555555555555";
        let url = client.automation_base_url(uuid);
        assert!(
            url.ends_with(uuid),
            "cloudId must appear verbatim at the end of the URL: {url}"
        );
        // No double slashes anywhere in the URL.
        assert!(
            !url.trim_start_matches("https://").contains("//"),
            "URL must not contain double slashes: {url}"
        );
    }

    #[test]
    fn automation_base_url_has_exactly_one_trailing_segment() {
        // Per-call code paths append `/rest/v1/...` to this base. If the
        // base URL itself already ends with a slash, we'd get a double-slash
        // joining bug.
        let client = make_dc_client();
        let url = client.automation_base_url("cid");
        assert!(
            !url.ends_with('/'),
            "automation_base_url must not have a trailing slash: {url}"
        );
    }

    #[test]
    fn automation_base_url_uses_override_when_set() {
        // The `with_automation_base_url` test seam must redirect the host
        // (scheme + authority) while keeping the `/automation/public/jira/{cid}`
        // suffix intact. Component tests rely on this to point automation
        // calls at a local mock server.
        let host = "http://127.0.0.1:38421";
        let client = make_dc_client().with_automation_base_url(host);
        let url = client.automation_base_url("abc");
        assert_eq!(url, format!("{host}/automation/public/jira/abc"));
    }
}

#[cfg(test)]
mod automation_component_tests {
    //! Component tests for `JiraClient` automation paths that need a real
    //! HTTP server. We use [`httpmock`] to stand up a fake site on
    //! `127.0.0.1:port` and aim the client at it via `instance.domain`.
    //!
    //! HTTP-shape coverage for the automation API uses the
    //! [`JiraClient::with_automation_base_url`] test seam to redirect the
    //! hardcoded `https://api.atlassian.com/automation/...` calls to the
    //! same mock server. Two mocks per test are therefore registered: one
    //! for `/_edge/tenant_info` (cloud-id resolution) and one for the
    //! actual `/automation/public/jira/<cloud_id>/...` path.
    //!
    //! Coverage:
    //!
    //! * `get_cloud_id` HTTP shape + memoisation + error mapping.
    //! * `read_only` enforcement on the mutating automation helpers — these
    //!   short-circuit in `assert_writable` before touching the network.
    //! * Full URL/method/header/body shape for the six automation methods:
    //!   `list`, `get`, `create`, `update`, `set_state`, `delete`.
    use super::*;
    use crate::auth::InMemoryStore;
    use crate::config::AuthType;
    use crate::test_util::env_lock;
    use httpmock::Method::{DELETE, GET, POST, PUT};
    use httpmock::MockServer;
    use pretty_assertions::assert_eq;

    /// A real-shape Jira Cloud `cloudId` (a UUIDv7-style string). Used in
    /// every test to make sure the path interpolation handles real values
    /// — not just `"abc-123"`.
    const TEST_CLOUD_ID: &str = "0192e5ac-0e25-71de-ba7b-d972e6a2049a";

    /// A real-shape automation rule UUID — same format the upstream
    /// returns. Used to assert the rule-uuid is interpolated verbatim into
    /// the URL path (no encoding, no normalisation).
    const TEST_RULE_UUID: &str = "0192e5ac-0e25-71de-ba7b-d972e6a2049a";

    /// Build a [`JiraClient`] aimed at a [`MockServer`]. Forces Cloud flavor so
    /// the automation paths apply.
    fn make_client_for(server: &MockServer) -> JiraClient {
        let _g = env_lock();
        // SAFETY: env access is serialized by env_lock() for the whole test.
        unsafe { std::env::remove_var("ATL_API_TOKEN") };
        let inst = AtlassianInstance {
            domain: server.base_url(),
            email: Some("alice@company.com".into()),
            api_token: Some("irrelevant".into()),
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: Some(JiraFlavor::Cloud),
        };
        let store = InMemoryStore::new();
        JiraClient::new(&inst, "default", &store, RetryConfig::off())
            .expect("JiraClient should build")
    }

    /// Same as [`make_client_for`] but redirects the automation API host
    /// to the same `MockServer` via the `with_automation_base_url` test
    /// seam. Use this for any test that asserts the HTTP shape of one of
    /// the six automation methods.
    fn make_automation_client_for(server: &MockServer) -> JiraClient {
        make_client_for(server).with_automation_base_url(server.base_url())
    }

    /// Register a mock for `/_edge/tenant_info` returning [`TEST_CLOUD_ID`].
    /// All automation tests need this so `get_cloud_id` resolves before
    /// the actual automation path is hit.
    async fn mock_tenant_info(server: &MockServer) -> httpmock::Mock<'_> {
        server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(serde_json::json!({"cloudId": TEST_CLOUD_ID}));
            })
            .await
    }

    /// Same as [`make_client_for`] but with `read_only = true`.
    fn make_readonly_client_for(server: &MockServer) -> JiraClient {
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };
        let inst = AtlassianInstance {
            domain: server.base_url(),
            email: Some("alice@company.com".into()),
            api_token: Some("irrelevant".into()),
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: true,
            flavor: Some(JiraFlavor::Cloud),
        };
        let store = InMemoryStore::new();
        JiraClient::new(&inst, "default", &store, RetryConfig::off())
            .expect("JiraClient should build")
    }

    // -- get_cloud_id --

    #[tokio::test]
    async fn get_cloud_id_returns_cloud_id_from_tenant_info() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(serde_json::json!({"cloudId": "abc-123"}));
            })
            .await;

        let client = make_client_for(&server);
        let cid = client.get_cloud_id().await.expect("should resolve cloudId");
        assert_eq!(cid, "abc-123");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_cloud_id_memoizes_after_first_call() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(200)
                    .json_body(serde_json::json!({"cloudId": "memo-id"}));
            })
            .await;

        let client = make_client_for(&server);
        let first = client.get_cloud_id().await.expect("first call");
        let second = client.get_cloud_id().await.expect("second call");
        assert_eq!(first, "memo-id");
        assert_eq!(second, "memo-id");
        // Only ONE upstream hit despite two client calls — the OnceCell did
        // its job.
        mock.assert_hits_async(1).await;
    }

    #[tokio::test]
    async fn get_cloud_id_404_is_config_error() {
        // Non-Cloud sites don't expose `/_edge/tenant_info`. The client must
        // surface a Config error with a hint about needing Cloud, not a
        // generic 404 NotFound.
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(404);
            })
            .await;

        let client = make_client_for(&server);
        let err = client
            .get_cloud_id()
            .await
            .expect_err("expected error, got success");
        match err {
            Error::Config(msg) => {
                assert!(
                    msg.contains("Cloud"),
                    "Config message should mention Cloud, got: {msg}"
                );
            }
            other => panic!("expected Error::Config, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_cloud_id_unauthorized_is_auth_error() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(401).body("nope");
            })
            .await;

        let client = make_client_for(&server);
        let err = client
            .get_cloud_id()
            .await
            .expect_err("expected auth error");
        assert!(
            matches!(err, Error::Auth(_)),
            "expected Error::Auth, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn get_cloud_id_500_is_api_error() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(500).body("boom");
            })
            .await;

        let client = make_client_for(&server);
        let err = client.get_cloud_id().await.expect_err("expected api error");
        match err {
            Error::Api { status, .. } => assert_eq!(status, 500),
            other => panic!("expected Error::Api, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_cloud_id_response_missing_field_is_invalid_response() {
        // tenant_info returned 200 but didn't include `cloudId`. We must
        // surface an InvalidResponse error so the operator knows the site
        // returned an unexpected shape.
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(200)
                    .json_body(serde_json::json!({"otherField": "x"}));
            })
            .await;

        let client = make_client_for(&server);
        let err = client
            .get_cloud_id()
            .await
            .expect_err("expected invalid-response error");
        assert!(
            matches!(err, Error::InvalidResponse(_)),
            "expected Error::InvalidResponse, got: {err:?}"
        );
    }

    // -- read_only enforcement on automation mutators --

    #[tokio::test]
    async fn create_automation_rule_blocks_read_only() {
        let server = MockServer::start_async().await;
        // No mock registered: if assert_writable lets the call through, the
        // mock server returns 404 / a different error and this test fails.
        let client = make_readonly_client_for(&server);
        let payload = serde_json::json!({"name": "rule"});
        let err = client
            .create_automation_rule(&payload)
            .await
            .expect_err("expected read-only error");
        match err {
            Error::Config(msg) => assert!(msg.contains("read-only"), "got: {msg}"),
            other => panic!("expected Error::Config, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_automation_rule_blocks_read_only() {
        let server = MockServer::start_async().await;
        let client = make_readonly_client_for(&server);
        let err = client
            .update_automation_rule("uuid", &serde_json::json!({}))
            .await
            .expect_err("expected read-only error");
        assert!(matches!(err, Error::Config(ref m) if m.contains("read-only")));
    }

    #[tokio::test]
    async fn set_automation_rule_state_blocks_read_only() {
        let server = MockServer::start_async().await;
        let client = make_readonly_client_for(&server);
        let err = client
            .set_automation_rule_state("uuid", true)
            .await
            .expect_err("expected read-only error");
        assert!(matches!(err, Error::Config(ref m) if m.contains("read-only")));
    }

    #[tokio::test]
    async fn delete_automation_rule_blocks_read_only() {
        let server = MockServer::start_async().await;
        let client = make_readonly_client_for(&server);
        let err = client
            .delete_automation_rule("uuid")
            .await
            .expect_err("expected read-only error");
        assert!(matches!(err, Error::Config(ref m) if m.contains("read-only")));
    }

    // -- list/get fail with Config error when site isn't Cloud --
    //
    // Cloud-id resolution must short-circuit BEFORE the cross-host
    // automation call. We simulate "non-Cloud" by having the mock return
    // 404 on `/_edge/tenant_info`. One test on `list` is enough as a
    // smoke — the same routing applies to all six methods because they
    // all start with `self.get_cloud_id().await?` (see
    // `automation_base_url` callers).

    #[tokio::test]
    async fn list_automation_rules_routes_through_cloud_id() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/_edge/tenant_info");
                then.status(404);
            })
            .await;

        let client = make_client_for(&server);
        let err = client
            .list_automation_rules(None, None)
            .await
            .expect_err("expected Config error from cloud-id resolution");
        assert!(matches!(err, Error::Config(ref m) if m.contains("Cloud")));
    }

    // -- list_automation_rules HTTP shape --

    #[tokio::test]
    async fn list_automation_rules_no_query_sends_get_with_basic_auth() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/summary"
                    ))
                    // No `limit` or `cursor` should appear in the
                    // querystring when both parameters are None. We use a
                    // matcher closure because httpmock 0.7 lacks a
                    // `query_param_missing` helper.
                    .matches(|req| {
                        req.query_params
                            .as_ref()
                            .is_none_or(|qs| !qs.iter().any(|(k, _)| k == "limit" || k == "cursor"))
                    })
                    // Basic auth header must be set even with the test seam
                    // pointing at localhost — this proves the auth layer
                    // isn't bypassed by the override.
                    .header_exists("authorization");
                then.status(200).json_body(serde_json::json!({
                    "data": [],
                    "links": {"next": null}
                }));
            })
            .await;

        let client = make_automation_client_for(&server);
        let v = client
            .list_automation_rules(None, None)
            .await
            .expect("list should succeed");
        assert_eq!(v["data"], serde_json::json!([]));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn list_automation_rules_with_limit_sends_limit_query() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/summary"
                    ))
                    .query_param("limit", "50");
                then.status(200).json_body(serde_json::json!({
                    "data": [{"id": 1, "name": "rule-a"}],
                    "links": {}
                }));
            })
            .await;

        let client = make_automation_client_for(&server);
        let v = client
            .list_automation_rules(None, Some(50))
            .await
            .expect("list with limit should succeed");
        assert_eq!(v["data"][0]["name"], "rule-a");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn list_automation_rules_with_cursor_sends_cursor_query() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/summary"
                    ))
                    .query_param("cursor", "next-page-token");
                then.status(200)
                    .json_body(serde_json::json!({"data": [], "links": {}}));
            })
            .await;

        let client = make_automation_client_for(&server);
        client
            .list_automation_rules(Some("next-page-token"), None)
            .await
            .expect("list with cursor should succeed");
        mock.assert_async().await;
    }

    // -- get_automation_rule HTTP shape --

    #[tokio::test]
    async fn get_automation_rule_interpolates_uuid_verbatim() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path(format!(
                    "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}"
                ));
                then.status(200).json_body(serde_json::json!({
                    "id": TEST_RULE_UUID,
                    "name": "Daily report",
                    "state": "ENABLED"
                }));
            })
            .await;

        let client = make_automation_client_for(&server);
        let rule = client
            .get_automation_rule(TEST_RULE_UUID)
            .await
            .expect("get should succeed");
        assert_eq!(rule["id"], TEST_RULE_UUID);
        assert_eq!(rule["name"], "Daily report");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_automation_rule_404_maps_to_not_found_error() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        server
            .mock_async(|when, then| {
                when.method(GET).path(format!(
                    "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}"
                ));
                then.status(404).body("rule not found");
            })
            .await;

        let client = make_automation_client_for(&server);
        let err = client
            .get_automation_rule(TEST_RULE_UUID)
            .await
            .expect_err("expected NotFound");
        // Per `exit_code_for_error`, NotFound maps to exit code 2.
        assert!(
            matches!(err, Error::NotFound(_)),
            "expected Error::NotFound, got: {err:?}"
        );
    }

    // -- create_automation_rule HTTP shape --

    #[tokio::test]
    async fn create_automation_rule_posts_json_body_round_trip() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule"
                    ))
                    .header("content-type", "application/json")
                    // The body must contain the `name` field verbatim — we
                    // assert the substring rather than full equality so the
                    // test isn't sensitive to JSON key ordering.
                    .body_contains("\"name\":\"My new rule\"")
                    .body_contains("\"trigger\"");
                then.status(201)
                    .header("content-type", "application/json")
                    .json_body(serde_json::json!({
                        "id": TEST_RULE_UUID,
                        "name": "My new rule"
                    }));
            })
            .await;

        let payload = serde_json::json!({
            "name": "My new rule",
            "trigger": {"component": "TRIGGER", "type": "scheduled"}
        });
        let client = make_automation_client_for(&server);
        let resp = client
            .create_automation_rule(&payload)
            .await
            .expect("create should succeed");
        assert_eq!(resp["id"], TEST_RULE_UUID);
        mock.assert_async().await;
    }

    // -- update_automation_rule HTTP shape --

    #[tokio::test]
    async fn update_automation_rule_puts_to_uuid_path_with_body() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}"
                    ))
                    .header("content-type", "application/json")
                    .body_contains("\"name\":\"Renamed rule\"");
                then.status(200)
                    .json_body(serde_json::json!({"id": TEST_RULE_UUID}));
            })
            .await;

        let payload = serde_json::json!({"name": "Renamed rule"});
        let client = make_automation_client_for(&server);
        client
            .update_automation_rule(TEST_RULE_UUID, &payload)
            .await
            .expect("update should succeed");
        mock.assert_async().await;
    }

    // -- set_automation_rule_state HTTP shape --

    #[tokio::test]
    async fn set_state_enable_sends_put_with_enabled_body() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}/state"
                    ))
                    .header("content-type", "application/json")
                    .json_body_partial(r#"{"state":"ENABLED"}"#);
                // Atlassian returns 204 No Content here in practice — make
                // sure the maybe-empty handler accepts that.
                then.status(204);
            })
            .await;

        let client = make_automation_client_for(&server);
        client
            .set_automation_rule_state(TEST_RULE_UUID, true)
            .await
            .expect("enable should succeed");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn set_state_disable_sends_put_with_disabled_body() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(PUT)
                    .path(format!(
                        "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}/state"
                    ))
                    .json_body_partial(r#"{"state":"DISABLED"}"#);
                then.status(204);
            })
            .await;

        let client = make_automation_client_for(&server);
        client
            .set_automation_rule_state(TEST_RULE_UUID, false)
            .await
            .expect("disable should succeed");
        mock.assert_async().await;
    }

    // -- delete_automation_rule HTTP shape --

    #[tokio::test]
    async fn delete_automation_rule_sends_delete_and_returns_ok_on_204() {
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        let mock = server
            .mock_async(|when, then| {
                when.method(DELETE).path(format!(
                    "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}"
                ));
                then.status(204);
            })
            .await;

        let client = make_automation_client_for(&server);
        client
            .delete_automation_rule(TEST_RULE_UUID)
            .await
            .expect("delete should succeed");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn delete_automation_rule_409_surfaces_api_error_with_body() {
        // Atlassian returns 409 if the rule wasn't disabled before delete.
        // This must surface as Error::Api with the upstream message
        // intact so operators see exactly why the call was rejected.
        let server = MockServer::start_async().await;
        let _t = mock_tenant_info(&server).await;
        server
            .mock_async(|when, then| {
                when.method(DELETE).path(format!(
                    "/automation/public/jira/{TEST_CLOUD_ID}/rest/v1/rule/{TEST_RULE_UUID}"
                ));
                then.status(409)
                    .body("rule must be disabled before deletion");
            })
            .await;

        let client = make_automation_client_for(&server);
        let err = client
            .delete_automation_rule(TEST_RULE_UUID)
            .await
            .expect_err("expected api error");
        match err {
            Error::Api { status, message } => {
                assert_eq!(status, 409);
                assert!(
                    message.contains("disabled"),
                    "upstream message should be preserved, got: {message}"
                );
            }
            other => panic!("expected Error::Api, got: {other:?}"),
        }
    }

    // -- list_fields HTTP shape --

    #[tokio::test]
    async fn list_fields_cloud_hits_v3_endpoint() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/rest/api/3/field");
                then.status(200).json_body(serde_json::json!([
                    {"id": "summary", "name": "Summary"}
                ]));
            })
            .await;

        let client = make_client_for(&server);
        let v = client
            .list_fields()
            .await
            .expect("list_fields should succeed");
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "summary");
        mock.assert_async().await;
    }

    // The pure URL-formatting and state-payload tests live in the sibling
    // `tests` module (`automation_base_url_*`, `automation_state_payload_*`),
    // and are independent of the HTTP layer.
}
