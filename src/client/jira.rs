use serde_json::Value;
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
    ) -> Result<Value, Error> {
        let url = format!("{}/field/{field_id}/context/projectmapping", self.base_url);
        debug!("GET {url} contextId={context_id}");
        let resp = self
            .http
            .get(&url)
            .query(&[("contextId", context_id)])
            .send()
            .await?;
        handle_response(resp).await
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
    ) -> Result<Value, Error> {
        let url = format!(
            "{}/field/{field_id}/context/issuetypemapping",
            self.base_url
        );
        debug!("GET {url} contextId={context_id}");
        let resp = self
            .http
            .get(&url)
            .query(&[("contextId", context_id)])
            .send()
            .await?;
        handle_response(resp).await
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
}
