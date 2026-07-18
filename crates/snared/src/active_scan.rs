//! Active scanner (§ Burp active scan) — sends crafted probes into each query
//! parameter and inspects the response for reflected-XSS and error-based SQLi
//! indicators. Reuses the repeater client; findings go to the shared scanner.

use std::sync::Arc;

use serde_json::{json, Value};
use snare_core::model::{Header, HttpRequest, Source};
use snare_core::scanner::{Scanner, Severity};
use snare_core::store::FlowStore;
use tokio::sync::broadcast;

use snare_core::model::FlowEvent;

use crate::repeater;

const XSS_MARKER: &str = "snareXSS9137";
/// Substrings that strongly suggest a database error surfaced in the response.
const SQL_ERRORS: &[&str] = &[
    "sql syntax",
    "mysql_fetch",
    "you have an error in your sql",
    "unclosed quotation mark",
    "quoted string not properly terminated",
    "ora-01756",
    "ora-00933",
    "sqlite3::",
    "sqlite error",
    "pg_query",
    "postgresql query failed",
    "syntax error at or near",
];

fn rebuild_url(base: &HttpRequest, new_query: &str) -> String {
    let default_port = (base.scheme == "https" && base.port == 443)
        || (base.scheme == "http" && base.port == 80);
    let authority = if default_port {
        base.host.clone()
    } else {
        format!("{}:{}", base.host, base.port)
    };
    let mut url = format!("{}://{}{}", base.scheme, authority, base.path);
    if !new_query.is_empty() {
        url.push('?');
        url.push_str(new_query);
    }
    url
}

/// Encode a query back from pairs, injecting `payload` into the parameter at
/// `target_idx`.
fn query_with(pairs: &[(String, String)], target_idx: usize, payload: &str) -> String {
    pairs
        .iter()
        .enumerate()
        .map(|(i, (k, v))| {
            let val = if i == target_idx { payload } else { v.as_str() };
            format!("{k}={val}")
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Active-scan every query parameter of `base`. Returns one row per probe.
pub async fn scan(
    store: &Arc<dyn FlowStore>,
    events: &broadcast::Sender<FlowEvent>,
    scanner: &Arc<Scanner>,
    base: &HttpRequest,
) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(query) = &base.query else {
        return vec![json!({ "note": "no query parameters to test" })];
    };
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    if pairs.is_empty() {
        return vec![json!({ "note": "no key=value parameters to test" })];
    }

    let headers: Vec<Header> = base.headers.clone();
    for (idx, (name, orig)) in pairs.iter().enumerate() {
        let probes: [(&str, String, Severity, &str, fn(&str) -> bool); 2] = [
            (
                "xss",
                format!("{orig}\"'><{XSS_MARKER}>"),
                Severity::High,
                "Reflected XSS",
                (|body: &str| body.contains(&format!("<{XSS_MARKER}>"))) as fn(&str) -> bool,
            ),
            (
                "sqli",
                format!("{orig}'"),
                Severity::High,
                "SQL error (possible injection)",
                (|body: &str| {
                    let low = body.to_ascii_lowercase();
                    SQL_ERRORS.iter().any(|e| low.contains(e))
                }) as fn(&str) -> bool,
            ),
        ];
        for (kind, payload, sev, title, check) in probes {
            let q = query_with(&pairs, idx, &payload);
            let url = rebuild_url(base, &q);
            match repeater::send(store, events, Source::Scanner, &base.method, &url, &headers, base.body.clone()).await {
                Ok(flow) => {
                    let body = flow
                        .response
                        .as_ref()
                        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                        .unwrap_or_default();
                    let hit = check(&body);
                    if hit {
                        let f = scanner.record(
                            flow.id,
                            sev,
                            format!("{title} in `{name}`"),
                            format!("Parameter `{name}` — payload `{payload}` triggered the {kind} indicator."),
                            base.host.clone(),
                        );
                        let _ = events.send(FlowEvent::Finding { finding: f });
                    }
                    out.push(json!({
                        "param": name, "check": kind, "flow_id": flow.id,
                        "status": flow.response.as_ref().map(|r| r.status), "hit": hit,
                    }));
                }
                Err(e) => out.push(json!({ "param": name, "check": kind, "error": e.to_string() })),
            }
        }
    }
    out
}
