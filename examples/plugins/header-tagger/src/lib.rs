//! Example Snare plugin: logs each request and tags it with a header.
//!
//! Demonstrates the P1 plugin ABI — the `on-request`/`on-response` hooks and the
//! host `log` capability. Build with `cargo build --release --target wasm32-wasip2`.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../crates/snare-plugin/wit",
});

use exports::snare::plugin::hooks::{Guest, HttpRequest, HttpResponse, ReqAction, RespAction};
use snare::plugin::host;

struct Plugin;

impl Guest for Plugin {
    fn name() -> String {
        "header-tagger".to_string()
    }

    fn on_request(mut req: HttpRequest) -> ReqAction {
        host::log("info", &format!("header-tagger: {} {}", req.method, req.url));
        req.headers
            .push(("X-Snare-Plugin".to_string(), "header-tagger".to_string()));
        ReqAction::Forward(req)
    }

    fn on_response(_resp: HttpResponse) -> RespAction {
        RespAction::Unchanged
    }
}

export!(Plugin);
