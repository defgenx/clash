//! Linear transport for workflow sharing: post a share document as one
//! comment on an issue, through Linear's GraphQL API.
//!
//! Two calls: the issue's id from its key (`ENG-42`), then `commentCreate`.
//! Linear comments are markdown, so the document goes as written. The API key
//! rides a 0600 curl config file (`secure_post`), never argv. Payload shaping
//! and response parsing are pure and unit-tested; the sends are thin.
//!
//! Nothing here decides *when* to post: sharing is an explicit user action
//! with a preview.

const API_URL: &str = "https://api.linear.app/graphql";

/// clash's own cap on one comment, with an explicit marker past it — a long
/// share is cut visibly rather than rejected.
const LINEAR_MAX_CHARS: usize = 64_000;

/// Pure: the issue-lookup request for `key`.
pub fn issue_lookup_payload(key: &str) -> String {
    serde_json::json!({
        "query": "query($id: String!) { issue(id: $id) { id } }",
        "variables": { "id": key },
    })
    .to_string()
}

/// Pure: the `commentCreate` request, truncated with a marker. Returns the
/// body and whether it was truncated.
pub fn comment_payload(issue_id: &str, markdown: &str) -> (String, bool) {
    let marker = "\n\n… (truncated)";
    let truncated = markdown.chars().count() > LINEAR_MAX_CHARS;
    let body = if truncated {
        let keep: String = markdown
            .chars()
            .take(LINEAR_MAX_CHARS - marker.chars().count())
            .collect();
        format!("{}{}", keep, marker)
    } else {
        markdown.to_string()
    };
    let json = serde_json::json!({
        "query": "mutation($issueId: String!, $body: String!) { \
                  commentCreate(input: { issueId: $issueId, body: $body }) { success } }",
        "variables": { "issueId": issue_id, "body": body },
    });
    (json.to_string(), truncated)
}

/// Pure: the first GraphQL error message of a response, if any. Linear
/// answers most errors with HTTP 200 and an `errors` array.
fn graphql_error(v: &serde_json::Value) -> Option<String> {
    v.get("errors")?.as_array()?.first().map(|e| {
        e.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error")
            .to_string()
    })
}

/// Pure: the issue id out of a lookup response.
pub fn parse_issue_id(response: &str, key: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(response).map_err(|e| format!("unreadable Linear reply: {}", e))?;
    if let Some(e) = graphql_error(&v) {
        return Err(format!("Linear: {}", e));
    }
    v.pointer("/data/issue/id")
        .and_then(|id| id.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("Linear has no issue {}", key))
}

/// Pure: whether a `commentCreate` response reports success.
pub fn parse_comment_created(response: &str) -> Result<(), String> {
    let v: serde_json::Value =
        serde_json::from_str(response).map_err(|e| format!("unreadable Linear reply: {}", e))?;
    if let Some(e) = graphql_error(&v) {
        return Err(format!("Linear: {}", e));
    }
    match v
        .pointer("/data/commentCreate/success")
        .and_then(|s| s.as_bool())
    {
        Some(true) => Ok(()),
        _ => Err("Linear did not create the comment".to_string()),
    }
}

/// POST `markdown` as a comment on the issue `key`. Blocking (callers wrap in
/// `spawn_blocking`). Returns whether the comment was truncated.
pub fn post_comment(api_key: &str, key: &str, markdown: &str) -> Result<bool, String> {
    let key = key.trim().to_ascii_uppercase();
    if !crate::application::trackers::valid_ticket_key(&key) {
        return Err(format!("not a ticket key: {:?}", key));
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("Linear API key not configured".to_string());
    }
    // It lands in a curl config file; quotes/newlines would break out of it.
    if api_key.contains('"') || api_key.chars().any(|c| c.is_control()) {
        return Err("the Linear API key contains unsupported characters".to_string());
    }
    let pid = std::process::id();
    let tmp_body = std::env::temp_dir().join(format!("clash-linear-{}.json", pid));
    let tmp_auth = std::env::temp_dir().join(format!("clash-linear-{}.curl", pid));
    let result = (|| {
        crate::infrastructure::secure_post::write_private(
            &tmp_auth,
            &format!("header = \"Authorization: {}\"\n", api_key),
        )?;
        std::fs::write(&tmp_body, issue_lookup_payload(&key)).map_err(|e| e.to_string())?;
        let reply =
            crate::infrastructure::secure_post::post_json(API_URL, &tmp_body, &tmp_auth, "Linear")?;
        let issue_id = parse_issue_id(&reply, &key)?;
        let (body, truncated) = comment_payload(&issue_id, markdown);
        std::fs::write(&tmp_body, body).map_err(|e| e.to_string())?;
        let reply =
            crate::infrastructure::secure_post::post_json(API_URL, &tmp_body, &tmp_auth, "Linear")?;
        parse_comment_created(&reply).map(|_| truncated)
    })();
    let _ = std::fs::remove_file(&tmp_body);
    let _ = std::fs::remove_file(&tmp_auth);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_carry_their_variables_not_interpolated_strings() {
        let lookup: serde_json::Value =
            serde_json::from_str(&issue_lookup_payload("ENG-4")).unwrap();
        assert_eq!(lookup["variables"]["id"], "ENG-4");
        let (body, truncated) = comment_payload("uuid-1", "a \"quoted\" body");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["variables"]["issueId"], "uuid-1");
        assert_eq!(v["variables"]["body"], "a \"quoted\" body");
        assert!(!truncated);
    }

    #[test]
    fn a_long_comment_is_cut_visibly() {
        let long = "x".repeat(LINEAR_MAX_CHARS + 10);
        let (body, truncated) = comment_payload("id", &long);
        assert!(truncated);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let sent = v["variables"]["body"].as_str().unwrap();
        assert_eq!(sent.chars().count(), LINEAR_MAX_CHARS);
        assert!(sent.ends_with("… (truncated)"));
    }

    #[test]
    fn responses_are_read_for_ids_errors_and_success() {
        assert_eq!(
            parse_issue_id(r#"{"data":{"issue":{"id":"abc"}}}"#, "ENG-1").unwrap(),
            "abc"
        );
        assert!(parse_issue_id(r#"{"data":{"issue":null}}"#, "ENG-1")
            .unwrap_err()
            .contains("no issue ENG-1"));
        assert_eq!(
            parse_issue_id(r#"{"errors":[{"message":"Entity not found"}]}"#, "ENG-1").unwrap_err(),
            "Linear: Entity not found"
        );
        assert!(parse_comment_created(r#"{"data":{"commentCreate":{"success":true}}}"#).is_ok());
        assert!(parse_comment_created(r#"{"data":{"commentCreate":{"success":false}}}"#).is_err());
    }

    #[test]
    fn bad_inputs_are_refused_before_any_call() {
        assert!(post_comment("k", "nope", "x").is_err());
        assert!(post_comment("", "ENG-1", "x").is_err());
        assert!(post_comment("a\"b", "ENG-1", "x").is_err());
    }
}
