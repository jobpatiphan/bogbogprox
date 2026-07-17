//! REST API (§8) — the surface every frontend and the MCP server can use.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use snare_core::store::{FlowQuery, FlowStore};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn FlowStore>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub search: Option<String>,
    pub host: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/flows", get(list_flows))
        .route("/api/v1/flows/:id", get(get_flow))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "service": "snared" }))
}

async fn stats(State(st): State<AppState>) -> Response {
    match st.store.count() {
        Ok(count) => Json(json!({ "flows": count })).into_response(),
        Err(e) => err(e),
    }
}

async fn list_flows(State(st): State<AppState>, Query(p): Query<ListParams>) -> Response {
    let q = FlowQuery {
        search: p.search,
        host: p.host,
        limit: p.limit.unwrap_or(200),
        offset: p.offset.unwrap_or(0),
    };
    match st.store.list_flows(&q) {
        Ok(flows) => Json(flows).into_response(),
        Err(e) => err(e),
    }
}

async fn get_flow(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match st.store.get_flow(id) {
        Ok(Some(flow)) => Json(flow).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
        Err(e) => err(e),
    }
}

fn err(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
        .into_response()
}
