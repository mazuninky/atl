use camino::Utf8Path;
use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::auth::SecretStore;
use crate::cli::commands::converters::body_content::BodyContent;
use crate::config::AtlassianInstance;
use crate::error::Error;

use super::{
    HttpClient, RetryConfig, build_base_url, build_http_client, detect_confluence_api_path,
    handle_error_status, handle_response, handle_response_maybe_empty,
};

/// Build the v2 `body` payload from a converted body value.
///
/// Storage XHTML lands in `body.storage.value`; ADF documents are sent as
/// stringified JSON in `body.atlas_doc_format.value` (the v2 contract is
/// that the value is a string, not a nested JSON object).
fn body_payload(content: &BodyContent) -> Value {
    match content {
        BodyContent::Storage(xhtml) => json!({
            "representation": "storage",
            "value": xhtml,
        }),
        BodyContent::Adf(adf) => json!({
            "representation": "atlas_doc_format",
            // ADF must be sent as a stringified JSON. The fallback is
            // unreachable for valid `Value`s but keeps `body_payload` total
            // so callers don't have to handle a serialisation error here.
            "value": serde_json::to_string(adf).unwrap_or_else(|_| "{}".to_string()),
        }),
    }
}

pub struct ConfluenceClient {
    http: HttpClient,
    /// A client without retry middleware, used for multipart/streaming
    /// requests that cannot be cloned for retries.
    no_retry_http: HttpClient,
    base_url: String,
    base_url_v2: String,
    read_only: bool,
}

impl ConfluenceClient {
    pub fn new(
        instance: &AtlassianInstance,
        profile: &str,
        store: &dyn SecretStore,
        cfg: RetryConfig,
    ) -> Result<Self, Error> {
        let http = build_http_client(instance, profile, "confluence", store, cfg)?;
        // Build a separate client without retry middleware for multipart
        // requests. Multipart bodies are streaming and cannot be cloned,
        // which the retry middleware requires.
        let no_retry_http = if cfg.retries == 0 {
            http.clone()
        } else {
            build_http_client(instance, profile, "confluence", store, RetryConfig::off())?
        };
        let base_url = build_base_url(instance, "/wiki/rest/api");
        // Derive v2 URL: if api_path is set, transform it; otherwise use default
        let base_url_v2 = if let Some(ref custom_path) = instance.api_path {
            let v2_path = custom_path.replace("/rest/api", "/api/v2");
            let domain = instance.domain.trim_end_matches('/');
            let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
                ""
            } else {
                "https://"
            };
            format!("{scheme}{domain}{v2_path}")
        } else {
            build_base_url(instance, "/wiki/api/v2")
        };
        Ok(Self {
            http,
            no_retry_http,
            base_url,
            base_url_v2,
            read_only: instance.read_only,
        })
    }

    /// Create client with auto-detection of API path when not configured.
    pub async fn connect(
        instance: &AtlassianInstance,
        profile: &str,
        store: &dyn SecretStore,
        cfg: RetryConfig,
    ) -> Result<Self, Error> {
        let http = build_http_client(instance, profile, "confluence", store, cfg)?;
        let no_retry_http = if cfg.retries == 0 {
            http.clone()
        } else {
            build_http_client(instance, profile, "confluence", store, RetryConfig::off())?
        };
        let (base_url, base_url_v2) = if let Some(ref custom_path) = instance.api_path {
            // api_path overrides the v1 base; derive v2 from it
            let v1 = build_base_url(instance, custom_path);
            let v2_path = custom_path.replace("/rest/api", "/api/v2");
            let domain = instance.domain.trim_end_matches('/');
            let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
                ""
            } else {
                "https://"
            };
            (v1, format!("{scheme}{domain}{v2_path}"))
        } else {
            let api_path = detect_confluence_api_path(&http, &instance.domain).await?;
            let v2_path = api_path.replace("/rest/api", "/api/v2");
            (
                build_base_url(instance, &api_path),
                build_base_url(instance, &v2_path),
            )
        };
        Ok(Self {
            http,
            no_retry_http,
            base_url,
            base_url_v2,
            read_only: instance.read_only,
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

    pub async fn get_page(
        &self,
        page_id: &str,
        format: &str,
        extra_expand: &[&str],
    ) -> Result<Value, Error> {
        let body_format = match format {
            "view" => "view",
            "atlas_doc_format" | "adf" => "atlas_doc_format",
            _ => "storage",
        };
        let mut params: Vec<(&str, &str)> = vec![("body-format", body_format)];
        for expand in extra_expand {
            match *expand {
                "metadata.labels" => params.push(("include-labels", "true")),
                "metadata.properties" => params.push(("include-properties", "true")),
                "operations" => params.push(("include-operations", "true")),
                "version" => params.push(("include-versions", "true")),
                "collaborators" => params.push(("include-collaborators", "true")),
                "metadata.currentuser.favourited" => {
                    params.push(("include-favorited-by-current-user-status", "true"));
                }
                other => {
                    warn!(
                        "expand value '{other}' is not supported by the Confluence v2 \
                         endpoint and will be silently dropped from the request"
                    );
                }
            }
        }
        let path = format!("/pages/{page_id}");
        self.get_v2(&path, &params).await
    }

    pub async fn search(&self, cql: &str, limit: u32) -> Result<Value, Error> {
        let url = format!("{}/content/search", self.base_url);
        debug!("GET {url} cql={cql} limit={limit}");
        let resp = self
            .http
            .get(&url)
            .query(&[("cql", cql), ("limit", &limit.to_string())])
            .send()
            .await?;
        handle_response(resp).await
    }

    /// Search with auto-pagination: fetch all matching results.
    pub async fn search_all(&self, cql: &str, page_size: u32) -> Result<Value, Error> {
        let mut all_results = Vec::new();
        let mut start = 0u32;
        loop {
            let url = format!("{}/content/search", self.base_url);
            debug!("GET {url} cql={cql} start={start} limit={page_size}");
            let resp = self
                .http
                .get(&url)
                .query(&[
                    ("cql", cql),
                    ("start", &start.to_string()),
                    ("limit", &page_size.to_string()),
                ])
                .send()
                .await?;
            let page: Value = handle_response(resp).await?;
            let size = page.get("size").and_then(Value::as_u64).unwrap_or(0) as u32;
            if let Some(results) = page.get("results").and_then(Value::as_array) {
                if results.is_empty() {
                    break;
                }
                all_results.extend(results.iter().cloned());
            } else {
                break;
            }
            if size < page_size {
                break;
            }
            start += size;
        }
        Ok(serde_json::json!({
            "results": all_results,
            "start": 0,
            "limit": all_results.len(),
            "size": all_results.len(),
        }))
    }

    pub async fn get_spaces(&self, limit: u32) -> Result<Value, Error> {
        self.get_v2("/spaces", &[("limit", &limit.to_string())])
            .await
    }

    pub async fn get_children(&self, page_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/pages/{page_id}/children"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn create_page(
        &self,
        space_key: &str,
        title: &str,
        body: &BodyContent,
        parent_id: Option<&str>,
        private: bool,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let space_id = self.resolve_space_key_to_id(space_key).await?;
        let status = if private { "draft" } else { "current" };
        let mut payload = serde_json::json!({
            "spaceId": space_id,
            "title": title,
            "status": status,
            "body": body_payload(body),
        });
        if let Some(pid) = parent_id {
            payload["parentId"] = serde_json::json!(pid);
        }
        self.post_v2("/pages", &payload).await
    }

    pub async fn update_page(
        &self,
        page_id: &str,
        title: &str,
        body: &BodyContent,
        version: u64,
        version_message: Option<&str>,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        // Preserve the existing status (`current` vs `draft`) so we don't
        // accidentally publish a draft on update. Fetch only what we need.
        let existing = self.get_v2(&format!("/pages/{page_id}"), &[]).await?;
        let status = existing
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("current")
            .to_string();
        let mut ver = serde_json::json!({ "number": version });
        if let Some(msg) = version_message {
            ver["message"] = serde_json::json!(msg);
        }
        let payload = serde_json::json!({
            "id": page_id,
            "status": status,
            "title": title,
            "body": body_payload(body),
            "version": ver
        });
        self.put_v2(&format!("/pages/{page_id}"), &payload).await
    }

    pub async fn delete_page(&self, page_id: &str, purge: bool, draft: bool) -> Result<(), Error> {
        self.assert_writable()?;
        let mut params: Vec<(&str, &str)> = Vec::new();
        if purge {
            params.push(("purge", "true"));
        }
        if draft {
            params.push(("draft", "true"));
        }
        self.delete_v2_with_params(&format!("/pages/{page_id}"), &params)
            .await
    }

    pub async fn get_page_info(&self, page_id: &str) -> Result<Value, Error> {
        let path = format!("/pages/{page_id}");
        self.get_v2(&path, &[("include-version", "true")]).await
    }

    pub async fn get_attachments(
        &self,
        page_id: &str,
        limit: u32,
        media_type: Option<&str>,
        filename: Option<&str>,
    ) -> Result<Value, Error> {
        let limit_str = limit.to_string();
        let mut params: Vec<(&str, &str)> = vec![("limit", &limit_str)];
        if let Some(mt) = media_type {
            params.push(("mediaType", mt));
        }
        if let Some(fn_) = filename {
            params.push(("filename", fn_));
        }
        let path = format!("/pages/{page_id}/attachments");
        self.get_v2(&path, &params).await
    }

    /// Fetch all attachments for a page using cursor-based pagination.
    pub async fn get_attachments_all(&self, page_id: &str, page_size: u32) -> Result<Value, Error> {
        let path = format!("/pages/{page_id}/attachments");
        self.paginate_v2(&path, &[], page_size).await
    }

    pub async fn get_comments(&self, page_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/pages/{page_id}/footer-comments"),
            &[("limit", &limit.to_string()), ("body-format", "storage")],
        )
        .await
    }

    pub async fn create_comment(
        &self,
        page_id: &str,
        body: &str,
        parent_comment_id: Option<&str>,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let mut payload = serde_json::json!({
            "pageId": page_id,
            "body": {
                "representation": "storage",
                "value": body
            }
        });
        if let Some(pid) = parent_comment_id {
            payload["parentCommentId"] = serde_json::json!(pid);
        }
        self.post_v2("/footer-comments", &payload).await
    }

    pub async fn delete_comment(&self, comment_id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/footer-comments/{comment_id}"))
            .await
    }

    pub async fn delete_attachment(&self, attachment_id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/attachments/{attachment_id}"))
            .await
    }

    pub async fn get_properties(&self, page_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/properties"), &[("limit", "250")])
            .await
    }

    pub async fn get_property(&self, page_id: &str, key: &str) -> Result<Value, Error> {
        let resp = self
            .get_v2(
                &format!("/pages/{page_id}/properties"),
                &[("key", key), ("limit", "1")],
            )
            .await?;
        resp.get("results")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("property '{key}' not found on page {page_id}")))
    }

    pub async fn set_property(
        &self,
        page_id: &str,
        key: &str,
        value: &Value,
    ) -> Result<Value, Error> {
        self.assert_writable()?;

        // Check existence via v2 filtered GET
        let existing = self
            .get_v2(
                &format!("/pages/{page_id}/properties"),
                &[("key", key), ("limit", "1")],
            )
            .await?;

        let results = existing
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if let Some(prop) = results.first() {
            // Update: extract property id and version, then PUT
            let property_id = prop
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidResponse("property missing 'id' field".into()))?;
            let version_number = prop
                .pointer("/version/number")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let payload = serde_json::json!({
                "key": key,
                "value": value,
                "version": { "number": version_number }
            });
            self.put_v2(
                &format!("/pages/{page_id}/properties/{property_id}"),
                &payload,
            )
            .await
        } else {
            // Create
            let payload = serde_json::json!({
                "key": key,
                "value": value
            });
            self.post_v2(&format!("/pages/{page_id}/properties"), &payload)
                .await
        }
    }

    pub async fn download_attachment(
        &self,
        page_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<u8>, Error> {
        let url = format!(
            "{}/content/{page_id}/child/attachment/{attachment_id}/download",
            self.base_url
        );
        debug!("GET {url} (download attachment)");
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp.bytes().await?.to_vec())
        } else {
            Err(handle_error_status(status.as_u16(), resp)
                .await
                .unwrap_err())
        }
    }

    pub async fn upload_attachment(
        &self,
        page_id: &str,
        file_path: &Utf8Path,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/content/{page_id}/child/attachment", self.base_url);
        let file_name = file_path.file_name().unwrap_or("attachment").to_string();
        let bytes = std::fs::read(file_path.as_std_path()).map_err(Error::Io)?;
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(Error::Http)?;
        // `minorEdit` is required by the Confluence REST API for attachment
        // uploads — omitting it causes a 400 response. The value is false by
        // default to send a notification to watchers on upload.
        let form = reqwest::multipart::Form::new()
            .text("minorEdit", "false")
            .part("file", part);
        debug!("POST {url} (upload attachment)");
        // Use the no-retry client: multipart bodies are streaming and cannot
        // be cloned, which the retry middleware requires for retries.
        let resp = self
            .no_retry_http
            .post(&url)
            .header("X-Atlassian-Token", HeaderValue::from_static("nocheck"))
            .multipart(form)
            .send()
            .await?;
        handle_response(resp).await
    }

    pub async fn get_children_recursive(
        &self,
        page_id: &str,
        depth: u32,
        limit: u32,
    ) -> Result<Value, Error> {
        let page = self.get_page_info(page_id).await?;
        let title = page
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled")
            .to_string();

        let children = if depth > 0 {
            self.fetch_children_tree(page_id, depth, limit).await?
        } else {
            vec![]
        };

        Ok(serde_json::json!({
            "id": page_id,
            "title": title,
            "_children": children,
        }))
    }

    fn fetch_children_tree<'a>(
        &'a self,
        page_id: &'a str,
        depth: u32,
        limit: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>, Error>> + Send + 'a>>
    {
        Box::pin(async move {
            let resp = self.get_children(page_id, limit).await?;
            let results = resp
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            let mut nodes = Vec::with_capacity(results.len());
            for child in &results {
                let child_id = child.get("id").and_then(Value::as_str).unwrap_or("");
                let child_title = child
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("untitled");

                let grandchildren = if depth > 1 && !child_id.is_empty() {
                    info!("Fetching children of '{child_title}' (depth {})", depth - 1);
                    self.fetch_children_tree(child_id, depth - 1, limit).await?
                } else {
                    vec![]
                };

                nodes.push(serde_json::json!({
                    "id": child_id,
                    "title": child_title,
                    "_children": grandchildren,
                }));
            }
            Ok(nodes)
        })
    }

    // -- Blog Posts --

    pub async fn list_blog_posts(&self, space: Option<&str>, limit: u32) -> Result<Value, Error> {
        let limit_str = limit.to_string();
        let mut params: Vec<(&str, &str)> = vec![("limit", &limit_str)];
        let space_id;
        if let Some(sk) = space {
            space_id = self.resolve_space_key_to_id(sk).await?;
            params.push(("space-id", &space_id));
        }
        self.get_v2("/blogposts", &params).await
    }

    pub async fn get_blog_post(
        &self,
        blog_id: &str,
        format: &str,
        extra_expand: &[&str],
    ) -> Result<Value, Error> {
        let body_format = match format {
            "view" => "view",
            "atlas_doc_format" | "adf" => "atlas_doc_format",
            _ => "storage",
        };
        let mut params: Vec<(&str, &str)> = vec![("body-format", body_format)];
        for expand in extra_expand {
            match *expand {
                "metadata.labels" => params.push(("include-labels", "true")),
                "metadata.properties" => params.push(("include-properties", "true")),
                "operations" => params.push(("include-operations", "true")),
                "version" => params.push(("include-versions", "true")),
                "collaborators" => params.push(("include-collaborators", "true")),
                "metadata.currentuser.favourited" => {
                    params.push(("include-favorited-by-current-user-status", "true"));
                }
                other => {
                    warn!(
                        "expand value '{other}' is not supported by the Confluence v2 \
                         endpoint and will be silently dropped from the request"
                    );
                }
            }
        }
        let path = format!("/blogposts/{blog_id}");
        self.get_v2(&path, &params).await
    }

    pub async fn create_blog_post(
        &self,
        space_key: &str,
        title: &str,
        body: &BodyContent,
        private: bool,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        let space_id = self.resolve_space_key_to_id(space_key).await?;
        let status = if private { "draft" } else { "current" };
        let payload = serde_json::json!({
            "spaceId": space_id,
            "title": title,
            "body": body_payload(body),
            "status": status
        });
        self.post_v2("/blogposts", &payload).await
    }

    pub async fn update_blog_post(
        &self,
        blog_id: &str,
        title: &str,
        body: &BodyContent,
        version: u64,
        version_message: Option<&str>,
    ) -> Result<Value, Error> {
        self.assert_writable()?;
        // Preserve the existing status to avoid accidentally publishing a
        // draft blog post during update.
        let existing = self.get_v2(&format!("/blogposts/{blog_id}"), &[]).await?;
        let status = existing
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("current")
            .to_string();
        let mut ver = serde_json::json!({ "number": version });
        if let Some(msg) = version_message {
            ver["message"] = serde_json::json!(msg);
        }
        let payload = serde_json::json!({
            "id": blog_id,
            "status": status,
            "title": title,
            "body": body_payload(body),
            "version": ver
        });
        let path = format!("/blogposts/{blog_id}");
        self.put_v2(&path, &payload).await
    }

    pub async fn delete_blog_post(
        &self,
        blog_id: &str,
        purge: bool,
        draft: bool,
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let mut params: Vec<(&str, &str)> = Vec::new();
        if purge {
            params.push(("purge", "true"));
        }
        if draft {
            params.push(("draft", "true"));
        }
        self.delete_v2_with_params(&format!("/blogposts/{blog_id}"), &params)
            .await
    }

    // -- Labels --

    pub async fn get_labels(&self, page_id: &str, prefix: Option<&str>) -> Result<Value, Error> {
        let mut params: Vec<(&str, &str)> = Vec::new();
        if let Some(p) = prefix {
            params.push(("prefix", p));
        }
        let path = format!("/pages/{page_id}/labels");
        self.get_v2(&path, &params).await
    }

    pub async fn add_labels(&self, page_id: &str, labels: &[String]) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = format!("{}/content/{page_id}/label", self.base_url);
        let payload: Vec<Value> = labels
            .iter()
            .map(|l| serde_json::json!({ "prefix": "global", "name": l }))
            .collect();
        debug!("POST {url}");
        let resp = self.http.post(&url).json(&payload).send().await?;
        handle_response(resp).await
    }

    pub async fn remove_label(&self, page_id: &str, label: &str) -> Result<(), Error> {
        self.assert_writable()?;
        let url = format!("{}/content/{page_id}/label/{label}", self.base_url);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    pub async fn delete_property(&self, page_id: &str, key: &str) -> Result<(), Error> {
        self.assert_writable()?;

        // Look up property ID by key
        let existing = self
            .get_v2(
                &format!("/pages/{page_id}/properties"),
                &[("key", key), ("limit", "1")],
            )
            .await?;

        let property_id = existing
            .get("results")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|p| p.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::NotFound(format!("property '{key}' not found on page {page_id}"))
            })?;

        self.delete_v2(&format!("/pages/{page_id}/properties/{property_id}"))
            .await
    }

    // =========================================================================
    // Confluence REST API v2 helpers
    // =========================================================================

    fn v2_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url_v2, path)
    }

    /// GET a v2 endpoint with query parameters.
    pub async fn get_v2(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, Error> {
        let url = self.v2_url(path);
        debug!("GET {url}");
        let resp = self.http.get(&url).query(params).send().await?;
        handle_response(resp).await
    }

    /// POST JSON to a v2 endpoint (write-gated).
    pub async fn post_v2(&self, path: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        self.post_v2_read(path, payload).await
    }

    /// POST JSON to a v2 endpoint (read-safe, no write gate).
    pub async fn post_v2_read(&self, path: &str, payload: &Value) -> Result<Value, Error> {
        let url = self.v2_url(path);
        debug!("POST {url}");
        let resp = self.http.post(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    /// PUT JSON to a v2 endpoint.
    pub async fn put_v2(&self, path: &str, payload: &Value) -> Result<Value, Error> {
        self.assert_writable()?;
        let url = self.v2_url(path);
        debug!("PUT {url}");
        let resp = self.http.put(&url).json(payload).send().await?;
        handle_response(resp).await
    }

    /// DELETE a v2 endpoint.
    pub async fn delete_v2(&self, path: &str) -> Result<(), Error> {
        self.delete_v2_with_params(path, &[]).await
    }

    /// DELETE a v2 endpoint with query parameters.
    pub async fn delete_v2_with_params(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<(), Error> {
        self.assert_writable()?;
        let url = self.v2_url(path);
        debug!("DELETE {url}");
        let resp = self.http.delete(&url).query(params).send().await?;
        handle_response_maybe_empty(resp).await?;
        Ok(())
    }

    /// Resolve a Confluence space key to its numeric space ID.
    ///
    /// If the input is already all-digits, returns it as-is (assumed to be a space ID).
    pub async fn resolve_space_key_to_id(&self, space_key: &str) -> Result<String, Error> {
        if space_key.chars().all(|c| c.is_ascii_digit()) {
            return Ok(space_key.to_string());
        }
        let resp = self
            .get_v2("/spaces", &[("keys", space_key), ("limit", "1")])
            .await?;
        resp.get("results")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|s| s.get("id"))
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| Error::NotFound(format!("space with key '{space_key}' not found")))
    }

    /// Fetch all results from a v2 endpoint using cursor-based pagination.
    ///
    /// Returns a synthetic envelope: `{ "results": [...], "size": N }`.
    async fn paginate_v2(
        &self,
        path: &str,
        params: &[(&str, &str)],
        page_size: u32,
    ) -> Result<Value, Error> {
        let mut all_results = Vec::new();
        let page_size_str = page_size.to_string();

        // First request
        let mut query: Vec<(&str, &str)> = params.to_vec();
        query.push(("limit", &page_size_str));
        let mut page: Value = self.get_v2(path, &query).await?;

        while let Some(results) = page.get("results").and_then(Value::as_array) {
            if results.is_empty() {
                break;
            }
            all_results.extend(results.iter().cloned());

            // Follow cursor from _links.next. Confluence Cloud REST v2 returns
            // it as a relative path (e.g. `/wiki/api/v2/spaces?cursor=...`),
            // so combine it with `_links.base` from the same response.
            let next_url = page
                .pointer("/_links/next")
                .and_then(Value::as_str)
                .map(String::from);
            match next_url {
                Some(next) => {
                    let full_url = if next.starts_with("http://") || next.starts_with("https://") {
                        next.clone()
                    } else {
                        let base = page
                            .pointer("/_links/base")
                            .and_then(Value::as_str)
                            .unwrap_or(&self.base_url_v2)
                            .trim_end_matches('/');
                        format!("{base}{next}")
                    };
                    debug!("Following cursor: {full_url}");
                    let resp = self.http.get(&full_url).send().await?;
                    page = handle_response(resp).await?;
                }
                None => break,
            }
        }

        Ok(serde_json::json!({
            "results": all_results,
            "size": all_results.len(),
        }))
    }

    // =========================================================================
    // Confluence REST API v2 — Spaces
    // =========================================================================

    pub async fn get_spaces_v2(&self, limit: u32) -> Result<Value, Error> {
        self.get_v2("/spaces", &[("limit", &limit.to_string())])
            .await
    }

    /// Get all spaces with auto-pagination (v2 API, cursor-based).
    pub async fn get_spaces_all(&self, page_size: u32) -> Result<Value, Error> {
        self.paginate_v2("/spaces", &[], page_size).await
    }

    pub async fn get_space_v2(&self, space_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/spaces/{space_id}"), &[]).await
    }

    pub async fn create_space_v2(
        &self,
        key: &str,
        name: &str,
        description: Option<&str>,
        private: bool,
        alias: Option<&str>,
        template_key: Option<&str>,
    ) -> Result<Value, Error> {
        let mut payload = serde_json::json!({ "key": key, "name": name });
        if let Some(desc) = description {
            payload["description"] = serde_json::json!({
                "plain": { "value": desc, "representation": "plain" }
            });
        }
        if private {
            payload["type"] = serde_json::json!("personal");
        }
        if let Some(a) = alias {
            payload["alias"] = serde_json::json!(a);
        }
        if let Some(tk) = template_key {
            payload["templateKey"] = serde_json::json!(tk);
        }
        self.post_v2("/spaces", &payload).await
    }

    pub async fn delete_space_v2(&self, space_id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/spaces/{space_id}")).await
    }

    pub async fn get_space_pages_v2(&self, space_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/pages"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_space_blogposts_v2(&self, space_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/blogposts"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_space_labels_v2(&self, space_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/labels"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_space_permissions_v2(
        &self,
        space_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/permissions"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    // =========================================================================
    // Confluence REST API v2 — Page extras
    // =========================================================================

    pub async fn get_page_versions_v2(&self, page_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/pages/{page_id}/versions"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_page_version_v2(&self, page_id: &str, version: u32) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/versions/{version}"), &[])
            .await
    }

    pub async fn get_page_likes_v2(&self, page_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/likes"), &[]).await
    }

    pub async fn get_page_likes_count_v2(&self, page_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/likes/count"), &[])
            .await
    }

    pub async fn get_page_likes_users_v2(&self, page_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/likes/users"), &[])
            .await
    }

    pub async fn get_page_operations_v2(&self, page_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/operations"), &[])
            .await
    }

    pub async fn list_pages_v2(
        &self,
        space_ids: Option<&[String]>,
        title: Option<&str>,
        status: Option<&str>,
        sort: Option<&str>,
        limit: u32,
    ) -> Result<Value, Error> {
        let limit_str = limit.to_string();
        let space_str;
        let mut query: Vec<(&str, &str)> = vec![("limit", &limit_str)];
        if let Some(ids) = space_ids {
            space_str = ids.join(",");
            query.push(("space-id", &space_str));
        }
        if let Some(t) = title {
            query.push(("title", t));
        }
        if let Some(s) = status {
            query.push(("status", s));
        }
        if let Some(s) = sort {
            query.push(("sort", s));
        }
        self.get_v2("/pages", &query).await
    }

    pub async fn update_page_title_v2(
        &self,
        page_id: &str,
        title: &str,
        version: u32,
    ) -> Result<Value, Error> {
        let payload = serde_json::json!({
            "id": page_id,
            "status": "current",
            "title": title,
            "version": { "number": version }
        });
        self.put_v2(&format!("/pages/{page_id}"), &payload).await
    }

    pub async fn get_page_custom_content_v2(
        &self,
        page_id: &str,
        content_type: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/pages/{page_id}/custom-content"),
            &[("type", content_type), ("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn redact_page_v2(&self, page_id: &str) -> Result<Value, Error> {
        self.post_v2(&format!("/pages/{page_id}/redact"), &serde_json::json!({}))
            .await
    }

    pub async fn get_page_ancestors_v2(&self, page_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/pages/{page_id}/ancestors"), &[])
            .await
    }

    pub async fn get_page_descendants_v2(&self, page_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/pages/{page_id}/descendants"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    // =========================================================================
    // Confluence REST API v2 — Blog Post extras
    // =========================================================================

    pub async fn get_blogpost_attachments_v2(
        &self,
        blog_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/blogposts/{blog_id}/attachments"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_blogpost_labels_v2(&self, blog_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/blogposts/{blog_id}/labels"), &[])
            .await
    }

    pub async fn get_blogpost_footer_comments_v2(
        &self,
        blog_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/blogposts/{blog_id}/footer-comments"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_blogpost_inline_comments_v2(
        &self,
        blog_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/blogposts/{blog_id}/inline-comments"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_blogpost_versions_v2(
        &self,
        blog_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/blogposts/{blog_id}/versions"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_blogpost_likes_v2(&self, blog_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/blogposts/{blog_id}/likes"), &[])
            .await
    }

    pub async fn get_blogpost_operations_v2(&self, blog_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/blogposts/{blog_id}/operations"), &[])
            .await
    }

    pub async fn get_blogpost_version_v2(
        &self,
        blog_id: &str,
        version: u32,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/blogposts/{blog_id}/versions/{version}"), &[])
            .await
    }

    pub async fn get_blogpost_likes_count_v2(&self, blog_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/blogposts/{blog_id}/likes/count"), &[])
            .await
    }

    pub async fn get_blogpost_likes_users_v2(&self, blog_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/blogposts/{blog_id}/likes/users"), &[])
            .await
    }

    pub async fn get_blogpost_custom_content_v2(
        &self,
        blog_id: &str,
        content_type: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/blogposts/{blog_id}/custom-content"),
            &[("type", content_type), ("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn redact_blogpost_v2(&self, blog_id: &str) -> Result<Value, Error> {
        self.post_v2(
            &format!("/blogposts/{blog_id}/redact"),
            &serde_json::json!({}),
        )
        .await
    }

    // =========================================================================
    // Confluence REST API v2 — Footer Comments
    // =========================================================================

    /// List footer comments on a page.
    ///
    /// `body_format` is forwarded to the v2 `body-format` query parameter
    /// — `"storage"` (XHTML), `"view"` (rendered HTML), or
    /// `"atlas_doc_format"` (ADF). The wire shape of the response mirrors
    /// what page endpoints return.
    pub async fn list_footer_comments_v2(
        &self,
        page_id: &str,
        limit: u32,
        body_format: &str,
    ) -> Result<Value, Error> {
        let limit_str = limit.to_string();
        self.get_v2(
            &format!("/pages/{page_id}/footer-comments"),
            &[("limit", &limit_str), ("body-format", body_format)],
        )
        .await
    }

    pub async fn get_footer_comment_v2(
        &self,
        comment_id: &str,
        body_format: &str,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/footer-comments/{comment_id}"),
            &[("body-format", body_format)],
        )
        .await
    }

    /// Create a footer comment on a page.
    ///
    /// Accepts either storage XHTML or ADF (via [`BodyContent`]). The
    /// payload is shaped the same way pages are — see [`body_payload`].
    pub async fn create_footer_comment_v2(
        &self,
        page_id: &str,
        body: &BodyContent,
    ) -> Result<Value, Error> {
        let payload = serde_json::json!({
            "pageId": page_id,
            "body": body_payload(body),
        });
        self.post_v2("/footer-comments", &payload).await
    }

    pub async fn update_footer_comment_v2(
        &self,
        comment_id: &str,
        body: &BodyContent,
        version: u32,
    ) -> Result<Value, Error> {
        let payload = serde_json::json!({
            "version": { "number": version },
            "body": body_payload(body),
        });
        self.put_v2(&format!("/footer-comments/{comment_id}"), &payload)
            .await
    }

    pub async fn delete_footer_comment_v2(&self, comment_id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/footer-comments/{comment_id}"))
            .await
    }

    pub async fn get_footer_comment_children_v2(
        &self,
        comment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/footer-comments/{comment_id}/children"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_footer_comment_versions_v2(
        &self,
        comment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/footer-comments/{comment_id}/versions"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_footer_comment_likes_v2(&self, comment_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/footer-comments/{comment_id}/likes"), &[])
            .await
    }

    pub async fn get_footer_comment_operations_v2(&self, comment_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/footer-comments/{comment_id}/operations"), &[])
            .await
    }

    pub async fn get_footer_comment_likes_count_v2(
        &self,
        comment_id: &str,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/footer-comments/{comment_id}/likes/count"), &[])
            .await
    }

    pub async fn get_footer_comment_likes_users_v2(
        &self,
        comment_id: &str,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/footer-comments/{comment_id}/likes/users"), &[])
            .await
    }

    pub async fn get_footer_comment_version_v2(
        &self,
        comment_id: &str,
        version: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/footer-comments/{comment_id}/versions/{version}"),
            &[],
        )
        .await
    }

    // =========================================================================
    // Confluence REST API v2 — Inline Comments
    // =========================================================================

    pub async fn list_inline_comments_v2(
        &self,
        page_id: &str,
        limit: u32,
        resolution_status: Option<&str>,
        body_format: &str,
    ) -> Result<Value, Error> {
        let limit_str = limit.to_string();
        let mut query: Vec<(&str, &str)> =
            vec![("limit", &limit_str), ("body-format", body_format)];
        if let Some(rs) = resolution_status {
            query.push(("resolution-status", rs));
        }
        self.get_v2(&format!("/pages/{page_id}/inline-comments"), &query)
            .await
    }

    pub async fn get_inline_comment_v2(
        &self,
        comment_id: &str,
        body_format: &str,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/inline-comments/{comment_id}"),
            &[("body-format", body_format)],
        )
        .await
    }

    pub async fn create_inline_comment_v2(
        &self,
        page_id: &str,
        body: &BodyContent,
        inline_marker_ref: &str,
        text_selection: Option<&str>,
    ) -> Result<Value, Error> {
        let mut payload = serde_json::json!({
            "pageId": page_id,
            "body": body_payload(body),
            "inlineCommentProperties": { "inlineMarkerRef": inline_marker_ref }
        });
        if let Some(sel) = text_selection {
            payload["inlineCommentProperties"]["textSelection"] = Value::String(sel.to_string());
        }
        self.post_v2("/inline-comments", &payload).await
    }

    pub async fn update_inline_comment_v2(
        &self,
        comment_id: &str,
        body: &BodyContent,
        version: u32,
        resolved: Option<bool>,
    ) -> Result<Value, Error> {
        let mut payload = serde_json::json!({
            "version": { "number": version },
            "body": body_payload(body),
        });
        if let Some(r) = resolved {
            payload["resolved"] = serde_json::json!(r);
        }
        self.put_v2(&format!("/inline-comments/{comment_id}"), &payload)
            .await
    }

    pub async fn delete_inline_comment_v2(&self, comment_id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/inline-comments/{comment_id}"))
            .await
    }

    pub async fn get_inline_comment_children_v2(
        &self,
        comment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/inline-comments/{comment_id}/children"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_inline_comment_versions_v2(
        &self,
        comment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/inline-comments/{comment_id}/versions"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_inline_comment_likes_v2(&self, comment_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/inline-comments/{comment_id}/likes"), &[])
            .await
    }

    pub async fn get_inline_comment_operations_v2(&self, comment_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/inline-comments/{comment_id}/operations"), &[])
            .await
    }

    pub async fn get_inline_comment_likes_count_v2(
        &self,
        comment_id: &str,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/inline-comments/{comment_id}/likes/count"), &[])
            .await
    }

    pub async fn get_inline_comment_likes_users_v2(
        &self,
        comment_id: &str,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/inline-comments/{comment_id}/likes/users"), &[])
            .await
    }

    pub async fn get_inline_comment_version_v2(
        &self,
        comment_id: &str,
        version: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/inline-comments/{comment_id}/versions/{version}"),
            &[],
        )
        .await
    }

    // =========================================================================
    // Confluence REST API v2 — Attachment extras
    // =========================================================================

    pub async fn get_attachment_v2(&self, attachment_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/attachments/{attachment_id}"), &[])
            .await
    }

    pub async fn get_attachment_labels_v2(
        &self,
        attachment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/attachments/{attachment_id}/labels"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_attachment_comments_v2(
        &self,
        attachment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/attachments/{attachment_id}/comments"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_attachment_operations_v2(&self, attachment_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/attachments/{attachment_id}/operations"), &[])
            .await
    }

    pub async fn get_attachment_versions_v2(
        &self,
        attachment_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/attachments/{attachment_id}/versions"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_attachment_version_v2(
        &self,
        attachment_id: &str,
        version: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/attachments/{attachment_id}/versions/{version}"),
            &[],
        )
        .await
    }

    pub async fn get_custom_content_version_v2(
        &self,
        id: &str,
        version: u32,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/custom-content/{id}/versions/{version}"), &[])
            .await
    }

    // =========================================================================
    // Confluence REST API v2 — Generic content types (whiteboards, databases, folders, smart-links)
    // =========================================================================

    pub async fn create_content_type_v2(
        &self,
        type_name: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.post_v2(&format!("/{type_name}"), payload).await
    }

    pub async fn get_content_type_v2(&self, type_name: &str, id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/{type_name}/{id}"), &[]).await
    }

    pub async fn delete_content_type_v2(&self, type_name: &str, id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/{type_name}/{id}")).await
    }

    pub async fn get_content_type_sub_v2(
        &self,
        type_name: &str,
        id: &str,
        sub_resource: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/{type_name}/{id}/{sub_resource}"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_content_type_property_v2(
        &self,
        type_name: &str,
        id: &str,
        key: &str,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/{type_name}/{id}/properties/{key}"), &[])
            .await
    }

    pub async fn set_content_type_property_v2(
        &self,
        type_name: &str,
        id: &str,
        key: &str,
        value: &Value,
    ) -> Result<Value, Error> {
        let payload = serde_json::json!({ "key": key, "value": value });
        self.put_v2(&format!("/{type_name}/{id}/properties/{key}"), &payload)
            .await
    }

    pub async fn delete_content_type_property_v2(
        &self,
        type_name: &str,
        id: &str,
        key: &str,
    ) -> Result<(), Error> {
        self.delete_v2(&format!("/{type_name}/{id}/properties/{key}"))
            .await
    }

    // =========================================================================
    // Confluence REST API v2 — Custom Content
    // =========================================================================

    pub async fn list_custom_content_v2(
        &self,
        content_type: Option<&str>,
        space_id: Option<&str>,
        limit: u32,
    ) -> Result<Value, Error> {
        let mut params = vec![("limit", limit.to_string())];
        if let Some(t) = content_type {
            params.push(("type", t.to_string()));
        }
        if let Some(s) = space_id {
            params.push(("space-id", s.to_string()));
        }
        let param_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.get_v2("/custom-content", &param_refs).await
    }

    pub async fn get_custom_content_v2(&self, id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/custom-content/{id}"), &[]).await
    }

    pub async fn create_custom_content_v2(&self, payload: &Value) -> Result<Value, Error> {
        self.post_v2("/custom-content", payload).await
    }

    pub async fn update_custom_content_v2(
        &self,
        id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.put_v2(&format!("/custom-content/{id}"), payload).await
    }

    pub async fn delete_custom_content_v2(&self, id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/custom-content/{id}")).await
    }

    // =========================================================================
    // Confluence REST API v2 — Tasks
    // =========================================================================

    pub async fn list_tasks_v2(&self, params: &[(&str, &str)]) -> Result<Value, Error> {
        self.get_v2("/tasks", params).await
    }

    pub async fn get_task_v2(&self, task_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/tasks/{task_id}"), &[]).await
    }

    pub async fn update_task_v2(&self, task_id: &str, status: &str) -> Result<Value, Error> {
        let payload = serde_json::json!({ "status": status });
        self.put_v2(&format!("/tasks/{task_id}"), &payload).await
    }

    // =========================================================================
    // Confluence REST API v2 — Admin & Configuration
    // =========================================================================

    pub async fn get_admin_key_v2(&self) -> Result<Value, Error> {
        self.get_v2("/admin-key", &[]).await
    }

    pub async fn enable_admin_key_v2(&self) -> Result<Value, Error> {
        self.post_v2("/admin-key/enable", &Value::Null).await
    }

    pub async fn disable_admin_key_v2(&self) -> Result<Value, Error> {
        self.post_v2("/admin-key/disable", &Value::Null).await
    }

    pub async fn list_classification_levels_v2(&self) -> Result<Value, Error> {
        self.get_v2("/classification-levels", &[]).await
    }

    pub async fn get_content_classification_v2(
        &self,
        type_name: &str,
        id: &str,
    ) -> Result<Value, Error> {
        self.get_v2(&format!("/{type_name}/{id}/classification-level"), &[])
            .await
    }

    pub async fn set_content_classification_v2(
        &self,
        type_name: &str,
        id: &str,
        classification_id: &str,
    ) -> Result<Value, Error> {
        let payload = serde_json::json!({ "id": classification_id });
        self.put_v2(&format!("/{type_name}/{id}/classification-level"), &payload)
            .await
    }

    pub async fn reset_content_classification_v2(
        &self,
        type_name: &str,
        id: &str,
    ) -> Result<(), Error> {
        self.post_v2(
            &format!("/{type_name}/{id}/classification-level/reset"),
            &Value::Null,
        )
        .await
        .map(|_| ())
    }

    pub async fn get_space_permissions_available_v2(&self) -> Result<Value, Error> {
        self.get_v2("/space-permissions/available", &[]).await
    }

    pub async fn get_space_content_labels_v2(
        &self,
        space_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/content/labels"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_space_custom_content_v2(
        &self,
        space_id: &str,
        content_type: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/custom-content"),
            &[("type", content_type), ("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_space_operations_v2(&self, space_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/spaces/{space_id}/operations"), &[])
            .await
    }

    pub async fn get_space_role_assignments_v2(
        &self,
        space_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/role-assignments"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn set_space_role_assignments_v2(
        &self,
        space_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.put_v2(&format!("/spaces/{space_id}/role-assignments"), payload)
            .await
    }

    // -- Labels (v2) --

    pub async fn get_label_pages_v2(&self, label_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/labels/{label_id}/pages"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_label_blogposts_v2(&self, label_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/labels/{label_id}/blogposts"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_label_attachments_v2(
        &self,
        label_id: &str,
        limit: u32,
    ) -> Result<Value, Error> {
        self.get_v2(
            &format!("/labels/{label_id}/attachments"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn list_space_roles_v2(&self, space_id: &str, limit: u32) -> Result<Value, Error> {
        self.get_v2(
            &format!("/spaces/{space_id}/roles"),
            &[("limit", &limit.to_string())],
        )
        .await
    }

    pub async fn get_space_role_v2(&self, space_id: &str, role_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/spaces/{space_id}/roles/{role_id}"), &[])
            .await
    }

    pub async fn create_space_role_v2(
        &self,
        space_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.post_v2(&format!("/spaces/{space_id}/roles"), payload)
            .await
    }

    pub async fn update_space_role_v2(
        &self,
        space_id: &str,
        role_id: &str,
        payload: &Value,
    ) -> Result<Value, Error> {
        self.put_v2(&format!("/spaces/{space_id}/roles/{role_id}"), payload)
            .await
    }

    pub async fn delete_space_role_v2(&self, space_id: &str, role_id: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/spaces/{space_id}/roles/{role_id}"))
            .await
    }

    pub async fn get_space_roles_mode_v2(&self, space_id: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/spaces/{space_id}/roles/mode"), &[])
            .await
    }

    // =========================================================================
    // Confluence REST API v2 — Users & Misc
    // =========================================================================

    pub async fn bulk_lookup_users_v2(&self, account_ids: &[String]) -> Result<Value, Error> {
        let payload = serde_json::json!({ "accountIds": account_ids });
        self.post_v2_read("/users-bulk", &payload).await
    }

    pub async fn check_user_access_by_email_v2(&self, email: &str) -> Result<Value, Error> {
        let payload = serde_json::json!({ "email": email });
        self.post_v2_read("/user/access/check-access-by-email", &payload)
            .await
    }

    pub async fn invite_users_v2(&self, emails: &[String]) -> Result<Value, Error> {
        // Spec exposes a single-email invite endpoint; iterate and return
        // a structured envelope so callers can see per-email responses
        // (and any partial successes).
        if emails.is_empty() {
            return Err(Error::InvalidInput(
                "invite_users_v2: emails list cannot be empty".into(),
            ));
        }
        let mut results = Vec::with_capacity(emails.len());
        for email in emails {
            let payload = serde_json::json!({ "email": email });
            let response = self
                .post_v2("/user/access/invite-by-email", &payload)
                .await?;
            results.push(serde_json::json!({
                "email": email,
                "response": response,
            }));
        }
        Ok(serde_json::json!({ "results": results }))
    }

    pub async fn convert_content_ids_v2(&self, ids: &[String]) -> Result<Value, Error> {
        // Confluence v2 endpoint resolves content IDs to their types; it
        // doesn't take a source/target representation.
        let payload = serde_json::json!({ "contentIds": ids });
        self.post_v2_read("/content/convert-ids-to-types", &payload)
            .await
    }

    /// List the calling app's properties.
    ///
    /// Confluence v2 exposes only `/app/properties` — the app context comes
    /// from the auth credentials, so callers cannot target another app.
    pub async fn list_app_properties_v2(&self) -> Result<Value, Error> {
        self.get_v2("/app/properties", &[]).await
    }

    pub async fn get_app_property_v2(&self, key: &str) -> Result<Value, Error> {
        self.get_v2(&format!("/app/properties/{key}"), &[]).await
    }

    pub async fn set_app_property_v2(&self, key: &str, value: &Value) -> Result<Value, Error> {
        self.put_v2(&format!("/app/properties/{key}"), value).await
    }

    pub async fn delete_app_property_v2(&self, key: &str) -> Result<(), Error> {
        self.delete_v2(&format!("/app/properties/{key}")).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- body_payload ----

    #[test]
    fn body_payload_storage_uses_storage_representation() {
        // Storage XHTML must travel as `body.value` with the `storage`
        // representation tag — the historical Confluence wire format.
        let payload = body_payload(&BodyContent::Storage("<p>hi</p>".into()));
        assert_eq!(
            payload,
            json!({"representation": "storage", "value": "<p>hi</p>"}),
            "storage payload must use representation=storage and a verbatim value"
        );
    }

    #[test]
    fn body_payload_adf_uses_atlas_doc_format_with_stringified_value() {
        // ADF documents are sent as stringified JSON — the v2 contract is
        // that `body.value` is a string, not a nested object.
        let adf = json!({"type": "doc", "version": 1, "content": []});
        let payload = body_payload(&BodyContent::Adf(adf.clone()));
        assert_eq!(
            payload.get("representation").and_then(Value::as_str),
            Some("atlas_doc_format"),
            "ADF payload must use representation=atlas_doc_format"
        );
        let value = payload
            .get("value")
            .and_then(Value::as_str)
            .expect("value field must be a string");
        // The string value must round-trip back to the original ADF JSON
        // — the stringification must be lossless.
        let parsed: Value = serde_json::from_str(value).expect("stringified ADF must parse");
        assert_eq!(
            parsed, adf,
            "stringified ADF must be lossless when parsed back"
        );
    }
}
