use super::{http_transport::IntegrationHttp, *};

pub(super) const OPERATION: &str = "github.pull_request";
const PAGE_SIZE: usize = 10;
const MAX_PAGES: usize = 3;

pub(super) struct PullRequestQuery {
    repository: String,
    number: u32,
}

impl PullRequestQuery {
    pub(super) fn parse(arguments: &Value) -> Result<Self, ExecutionError> {
        let arguments = arguments
            .as_object()
            .ok_or_else(|| execution("GitHub pull request requires object arguments"))?;
        let parse_repository_name = |field| {
            let name = required_string(arguments, field)?;
            if name.len() > 100
                || matches!(name, "." | "..")
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            {
                return Err(execution(format!("invalid GitHub repository {field}")));
            }
            Ok(name)
        };
        let repository = format!(
            "{}/{}",
            parse_repository_name("owner")?,
            parse_repository_name("repo")?
        );
        let number = arguments
            .get("number")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| *number > 0)
            .ok_or_else(|| execution("invalid GitHub pull request number"))?;
        Ok(Self { repository, number })
    }

    pub(super) fn url(&self, connection: &IntegrationConnection) -> Result<Url, ExecutionError> {
        native_url(
            connection,
            &format!("/repos/{}/pulls/{}", self.repository, self.number),
        )
    }

    fn link(&self, url: &Url) -> Result<String, ExecutionError> {
        let mut link = url.clone();
        let endpoint = format!("/repos/{}/pulls/{}", self.repository, self.number);
        let api_path = url
            .path()
            .strip_suffix(&endpoint)
            .ok_or_else(|| execution("invalid GitHub pull request API path"))?;
        let web_path = api_path.strip_suffix("/api/v3").unwrap_or(api_path);
        if link.host_str() == Some("api.github.com") {
            link.set_host(Some("github.com")).map_err(execution)?;
        }
        link.set_path(&format!(
            "{web_path}/{}/pull/{}",
            self.repository, self.number
        ));
        Ok(link.into())
    }

    pub(super) async fn observe(
        &self,
        connection: &IntegrationConnection,
        http: &IntegrationHttp<'_>,
        output_limit: usize,
    ) -> Result<Value, ExecutionError> {
        let url = self.url(connection)?;
        let reader = GitHubReader {
            connection,
            http,
            body_limit: output_limit.saturating_sub(1_024),
        };
        let pull = reader
            .read(url.clone())
            .await
            .and_then(|page| self.parse_pull(&page.value, http))
            .map_err(|reason| execution(format!("GitHub pull request unavailable ({reason})")))?;
        let path = format!("/repos/{}/commits/{}", self.repository, pull.head_sha);
        let mut checks = reader
            .collect(&path, EvidenceKind::Checks, &pull.head_sha)
            .await;
        let mut statuses = reader
            .collect(&path, EvidenceKind::Statuses, &pull.head_sha)
            .await;
        if !pull.same_repository {
            checks.mark("fork_or_unknown_head_repository");
            statuses.mark("fork_or_unknown_head_repository");
        }

        let verification = reader
            .read(url.clone())
            .await
            .and_then(|page| self.parse_pull(&page.value, http));
        let (freshness, verification_issue) = match verification {
            Ok(latest) if latest == pull => ("verified", None),
            Ok(_) => ("stale", Some("pull_request_changed")),
            Err(reason) => ("unverified", Some(reason)),
        };
        let metadata = json!({
            "number": self.number,
            "url": http.redact_text(&self.link(&url)?),
            "state": pull.state,
            "draft": pull.draft,
            "observed_head_sha": pull.head_sha,
            "freshness": freshness,
            "verification_issue": verification_issue,
        });
        bounded_observation(metadata, &mut checks, &mut statuses, output_limit)
    }

    fn parse_pull(
        &self,
        value: &Value,
        http: &IntegrationHttp<'_>,
    ) -> Result<PullDetails, &'static str> {
        let state = match value.get("state").and_then(Value::as_str) {
            Some("open") => "open",
            Some("closed") if value.get("merged").and_then(Value::as_bool) == Some(true) => {
                "merged"
            }
            Some("closed") => "closed",
            _ => return Err("invalid_response"),
        };
        if value.get("number").and_then(Value::as_u64) != Some(u64::from(self.number)) {
            return Err("invalid_response");
        }
        let draft = value
            .get("draft")
            .and_then(Value::as_bool)
            .ok_or("invalid_response")?;
        let head_sha = valid_sha(value.pointer("/head/sha"))
            .ok_or("invalid_response")?
            .to_ascii_lowercase();
        if http.redact_text(&head_sha) != head_sha {
            return Err("invalid_response");
        }
        let same_repository = value
            .pointer("/head/repo/full_name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case(&self.repository));
        Ok(PullDetails {
            state,
            draft,
            head_sha,
            same_repository,
        })
    }
}

#[derive(PartialEq, Eq)]
struct PullDetails {
    state: &'static str,
    draft: bool,
    head_sha: String,
    same_repository: bool,
}

struct GitHubReader<'a, 'permit> {
    connection: &'a IntegrationConnection,
    http: &'a IntegrationHttp<'permit>,
    body_limit: usize,
}

struct Page {
    value: Value,
    next: Option<bool>,
}

impl GitHubReader<'_, '_> {
    async fn read(&self, url: Url) -> Result<Page, &'static str> {
        let response = self
            .http
            .send(PreparedHttpRequest {
                method: reqwest::Method::GET,
                url,
                body: None,
            })
            .await
            .map_err(|_| "read_failed")?;
        let headers = response.headers();
        let rate_limited = response.status().as_u16() == 429
            || (response.status().as_u16() == 403
                && (headers
                    .get("x-ratelimit-remaining")
                    .is_some_and(|value| value == "0")
                    || headers.contains_key("retry-after")));
        if rate_limited {
            return Err("rate_limited");
        }
        match response.status().as_u16() {
            200 => {}
            401 | 403 => return Err("access_denied"),
            404 => return Err("not_found"),
            _ => return Err("request_failed"),
        }
        let next = continuation_hint(headers);
        let bytes = bounded_response(response, self.body_limit)
            .await
            .map_err(|_| "response_unavailable_or_oversize")?;
        let value = serde_json::from_slice(&bytes).map_err(|_| "invalid_response")?;
        Ok(Page { value, next })
    }

    async fn collect(&self, path: &str, kind: EvidenceKind, head: &str) -> Evidence {
        let mut section = Evidence::default();
        let mut identities = BTreeSet::new();
        let mut observed_count = 0;
        for page_number in 1..=MAX_PAGES {
            let page = async {
                let mut url = native_url(self.connection, &format!("{path}/{}", kind.endpoint()))
                    .map_err(|_| "invalid_destination")?;
                append_pairs(
                    &mut url,
                    vec![
                        ("per_page", PAGE_SIZE.to_string()),
                        ("page", page_number.to_string()),
                    ],
                );
                self.read(url).await
            }
            .await;
            let page = match page {
                Ok(page) => page,
                Err(reason) => {
                    section.mark(reason);
                    break;
                }
            };
            if matches!(kind, EvidenceKind::Statuses)
                && !valid_sha(page.value.get("sha"))
                    .is_some_and(|sha| sha.eq_ignore_ascii_case(head))
            {
                section.mark("commit_mismatch");
                break;
            }
            let (Some(total), Some(rows)) = (
                page.value.get("total_count").and_then(Value::as_u64),
                page.value.get(kind.rows_field()).and_then(Value::as_array),
            ) else {
                section.mark("invalid_response");
                break;
            };
            if section.total.is_some_and(|previous| total != previous) {
                section.mark("pagination_changed");
            }
            section.total = Some(total);
            observed_count += rows.len().min(PAGE_SIZE) as u64;
            if rows.len() > PAGE_SIZE || observed_count > total {
                section.mark("invalid_response");
            }
            for row in rows.iter().take(PAGE_SIZE) {
                match kind.project(row, head, self.http) {
                    Ok((id, item)) if identities.insert(id) => section.items.push(item),
                    Ok(_) => section.mark("pagination_changed"),
                    Err(reason) => section.mark(reason),
                }
            }
            let Some(next) = page.next else {
                section.mark("invalid_pagination");
                break;
            };
            if next && observed_count >= total {
                section.mark("pagination_inconsistent");
            }
            if !next && observed_count >= total {
                break;
            }
            if rows.len() < PAGE_SIZE {
                section.mark("pagination_incomplete");
                break;
            }
            if page_number == MAX_PAGES {
                section.mark("page_limit");
            }
        }
        section
    }
}

// The provider's Link header is only a continuation hint. Never take a URL
// from it. GitHub emits angle-bracketed targets with link relation parameters.
fn continuation_hint(headers: &reqwest::header::HeaderMap) -> Option<bool> {
    let mut next = false;
    for value in headers.get_all("link") {
        let (leading, links) = value.to_str().ok()?.split_once('<')?;
        if !leading.trim().is_empty() {
            return None;
        }
        let mut links = links.split('<').peekable();
        while let Some(link) = links.next() {
            let (_, parameters) = link.split_once('>')?;
            let parameters = parameters.trim_end();
            let parameters = if links.peek().is_some() {
                parameters.strip_suffix(',')?
            } else {
                parameters
            };
            let relation = parameters
                .split(';')
                .filter_map(|part| part.split_once('='))
                .find_map(|(name, value)| {
                    name.trim().eq_ignore_ascii_case("rel").then_some(value)
                })?;
            let relation = relation.trim();
            let relation = if let Some(quoted) = relation.strip_prefix('"') {
                quoted.strip_suffix('"')?
            } else if relation.bytes().any(|byte| byte.is_ascii_whitespace()) {
                return None;
            } else {
                relation
            };
            if relation.trim().is_empty() || relation.contains([',', '"']) {
                return None;
            }
            next |= relation
                .split_ascii_whitespace()
                .any(|name| name.eq_ignore_ascii_case("next"));
        }
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_hints_support_quoted_and_unquoted_next_and_reject_ambiguous_headers() {
        let interpret = |header: &'static str| {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("link", HeaderValue::from_static(header));
            continuation_hint(&headers)
        };
        assert_eq!(
            interpret(
                r#"<https://elsewhere.test/path?part=a,b>; rel="next"; type="text/html", <https://elsewhere.test/last>; rel="last""#
            ),
            Some(true)
        );
        assert_eq!(
            interpret("<https://elsewhere.test/next>; rel=next"),
            Some(true)
        );
        assert_eq!(
            interpret("<https://elsewhere.test/back>; rel=prev"),
            Some(false)
        );
        assert_eq!(interpret("<https://elsewhere.test/next>; rel=\"next"), None);
        assert_eq!(
            interpret("<https://elsewhere.test/next>; rel=next, garbage"),
            None
        );
        assert_eq!(interpret(""), None);
    }

    #[test]
    fn pull_request_links_preserve_the_configured_enterprise_mount() {
        let query = PullRequestQuery::parse(&json!({"owner":"acme","repo":"tools","number":42}))
            .expect("valid query");
        let url = Url::parse("https://git.example.test/github/api/v3/repos/acme/tools/pulls/42")
            .expect("valid URL");
        assert_eq!(
            query.link(&url).expect("pull link"),
            "https://git.example.test/github/acme/tools/pull/42"
        );
        assert!(
            PullRequestQuery::parse(&json!({"owner":"..","repo":"tools","number":42})).is_err()
        );
    }
}

#[derive(Clone, Copy)]
enum EvidenceKind {
    Checks,
    Statuses,
}

impl EvidenceKind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Checks => "check-runs",
            Self::Statuses => "status",
        }
    }

    fn rows_field(self) -> &'static str {
        match self {
            Self::Checks => "check_runs",
            Self::Statuses => "statuses",
        }
    }

    fn project(
        self,
        row: &Value,
        head: &str,
        http: &IntegrationHttp<'_>,
    ) -> Result<(u64, Value), &'static str> {
        let id = row.get("id").and_then(Value::as_u64).ok_or("invalid_row")?;
        let item = match self {
            Self::Checks => {
                if !valid_sha(row.get("head_sha")).is_some_and(|sha| sha.eq_ignore_ascii_case(head))
                {
                    return Err("commit_mismatch");
                }
                let status = match row.get("status").and_then(Value::as_str) {
                    Some(
                        status @ ("queued" | "in_progress" | "completed" | "waiting" | "requested"
                        | "pending"),
                    ) => status,
                    _ => return Err("invalid_row"),
                };
                let conclusion = match row.get("conclusion") {
                    Some(value) if value.is_string() => {
                        Some(required_label(row, "conclusion", http)?)
                    }
                    None | Some(Value::Null) if status != "completed" => None,
                    _ => return Err("invalid_row"),
                };
                json!({
                    "id": id,
                    "name": required_label(row, "name", http)?,
                    "status": status,
                    "conclusion": conclusion,
                    "pending": status != "completed",
                })
            }
            Self::Statuses => {
                let state = match row.get("state").and_then(Value::as_str) {
                    Some(state @ ("success" | "pending" | "failure" | "error")) => state,
                    _ => return Err("invalid_row"),
                };
                json!({
                    "id": id,
                    "context": required_label(row, "context", http)?,
                    "state": state,
                    "pending": state == "pending",
                })
            }
        };
        Ok((id, item))
    }
}

fn valid_sha(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn required_label(
    value: &Value,
    key: &str,
    http: &IntegrationHttp<'_>,
) -> Result<String, &'static str> {
    let label = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or("invalid_row")?;
    let label = http.redact_text(&label.replace(char::is_control, ""));
    if label.is_empty() {
        return Err("invalid_row");
    }
    Ok(label[..label.floor_char_boundary(128)].into())
}

#[derive(Default, Serialize)]
struct Evidence {
    total: Option<u64>,
    items: Vec<Value>,
    incomplete: BTreeSet<&'static str>,
}

impl Evidence {
    fn mark(&mut self, reason: &'static str) {
        self.incomplete.insert(reason);
    }
}

fn bounded_observation(
    mut metadata: Value,
    checks: &mut Evidence,
    statuses: &mut Evidence,
    limit: usize,
) -> Result<Value, ExecutionError> {
    let has_pending = checks
        .items
        .iter()
        .chain(&statuses.items)
        .any(|item| item.get("pending").and_then(Value::as_bool) == Some(true));
    loop {
        let state = if metadata["freshness"] == "stale" {
            "stale"
        } else if metadata["freshness"] != "verified"
            || !checks.incomplete.is_empty()
            || !statuses.incomplete.is_empty()
        {
            "incomplete"
        } else if checks.items.is_empty() && statuses.items.is_empty() {
            "empty"
        } else {
            "complete"
        };
        metadata["evidence_state"] = json!(state);
        metadata["has_observed_pending"] = json!(has_pending);
        metadata["check_runs"] = json!(checks);
        metadata["commit_statuses"] = json!(statuses);
        let result = json!({
            "status_code": 200,
            "content_type": "application/json",
            "result": metadata,
        });
        if serde_json::to_vec(&result).map_err(execution)?.len() <= limit {
            return Ok(result);
        }
        if checks.items.is_empty() && statuses.items.is_empty() {
            return Err(execution("GitHub observation exceeds its output bound"));
        }
        if checks.items.len() >= statuses.items.len() {
            checks.items.pop();
            checks.mark("output_limit");
        } else {
            statuses.items.pop();
            statuses.mark("output_limit");
        }
    }
}
