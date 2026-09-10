//! Guarded route composition — the RECOMMENDED way to mount the deal module.
//!
//! Hand-authored (user-owned; see `metaphor.codegen.yaml`). Opportunities are read + **verbs**;
//! the generic create/update/delete CRUD is NOT mounted for them, so a caller cannot hand-set the
//! derived `status`, clobber a manual probability, or write a `quotation_id` — the computed
//! lifecycle fields move only through the gated stage/win/lose statements. Stages, by contrast,
//! are tenant config masters with no cross-entity invariants, so their generic (scope-fenced)
//! writes ARE mounted — the recruitment module's posture for its stage config.
//!
//! Tenancy (ADR-0029): the module carries no tenancy of its own. `org_auth` verifies the Bearer
//! token, resolves the session's org scope against the request's tenant tree, and runs every
//! handler inside that scope — the module's statements ride the request-dedicated connection it
//! binds, and the composing service's tenancy decorator does the actual row-level fencing. The
//! guard reads the tenant database from the `backbone_orm::PgPool` request extension, so this
//! surface must be mounted inside the composing service's tenant router (the same wiring every
//! org-guarded module requires).
//!
//! The win verb needs the selling handoff (`SellingPort`) and the outcome events (`DealEventSink`);
//! a composing service supplies both (the adapters live in the composing app, not here).
//!
//! `DealWriteService` is stateless over the pool, so it is constructed here rather than pulled
//! from the generated `DealModule` struct — the guarded surface survives a regen of the module.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware::from_fn_with_state,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use backbone_auth::org::{org_auth, OrgVerifier};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::create_stage_write_routes;
use crate::application::service::deal_ports::SellingPort;
use crate::application::service::deal_write_service::{DealError, DealWriteService};
use crate::domain::event::DealEventSink;
use crate::DealModule;

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdResponse {
    id: Uuid,
}
fn err_response(e: DealError) -> axum::response::Response {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(ErrorBody { error: e.code(), message: e.to_string() })).into_response()
}

// ── request/response bodies ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveStageBody {
    stage_id: Uuid,
    // Optional explicit probability edit (0..100). Omitted = keep the current manual
    // probability; on a move onto a won stage the forced 100 wins regardless.
    #[serde(default)]
    probability: Option<Decimal>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MoveStageResponse {
    id: Uuid,
    status: String,
    probability: Decimal,
    date_closed: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WinResponse {
    quotation_id: Uuid,
    amount: Decimal,
    already: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoseBody {
    #[serde(default)]
    lost_reason: Option<String>,
    #[serde(default)]
    competitor: Option<String>,
}

// ── handlers ─────────────────────────────────────────────────────────────────

// All three verbs are ID-only: the scope is the one `org_auth` resolved and bound for the
// request (the gated statements ride the request-dedicated connection it holds), never a
// body field — a client must not be able to name the tenant it writes into.

async fn move_stage(
    State(deps): State<Arc<VerbDeps>>,
    Path(id): Path<Uuid>,
    Json(b): Json<MoveStageBody>,
) -> axum::response::Response {
    match deps.service.advance_stage(id, b.stage_id, b.probability).await {
        Ok(m) => (
            StatusCode::OK,
            Json(MoveStageResponse {
                id,
                status: m.status,
                probability: m.probability,
                date_closed: m.date_closed,
            }),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

async fn win(State(deps): State<Arc<VerbDeps>>, Path(id): Path<Uuid>) -> axum::response::Response {
    match deps
        .service
        .win_opportunity(id, deps.selling.as_ref(), deps.sink.as_ref())
        .await
    {
        Ok(w) => (
            StatusCode::OK,
            Json(WinResponse { quotation_id: w.quotation_id, amount: w.amount, already: w.already }),
        )
            .into_response(),
        Err(e) => err_response(e),
    }
}

async fn lose(
    State(deps): State<Arc<VerbDeps>>,
    Path(id): Path<Uuid>,
    Json(b): Json<LoseBody>,
) -> axum::response::Response {
    match deps.service.lose_opportunity(id, b.lost_reason, b.competitor, deps.sink.as_ref()).await {
        Ok(()) => (StatusCode::OK, Json(IdResponse { id })).into_response(),
        Err(e) => err_response(e),
    }
}

/// The verb handlers' shared state: the write service plus the two seams a composing
/// service supplies (the selling adapter and the outcome-event sink).
struct VerbDeps {
    service: Arc<DealWriteService>,
    selling: Arc<dyn SellingPort>,
    sink: Arc<dyn DealEventSink>,
}

fn create_deal_verb_routes(
    service: Arc<DealWriteService>,
    selling: Arc<dyn SellingPort>,
    sink: Arc<dyn DealEventSink>,
    verifier: OrgVerifier,
) -> Router {
    let deps = Arc::new(VerbDeps { service, selling, sink });
    Router::new()
        .route("/opportunities/:id/stage", post(move_stage))
        .route("/opportunities/:id/win", post(win))
        .route("/opportunities/:id/lose", post(lose))
        // Every write above is scope-bound: `org_auth` rejects a request whose token is absent,
        // invalid, or names a unit outside this tenant's tree, and runs the handler inside the
        // resolved org scope — a handler only ever executes with a proven, fenced session.
        //
        // `route_layer`, not `layer`: `layer` would also wrap this router's fallback, so once merged
        // every *unmatched* path (e.g. the generic CRUD paths this surface deliberately does not
        // mount) would answer 401 instead of 404 — leaking "auth required" for routes that do not
        // exist, and masking the CRUD-bypass probes.
        .route_layer(from_fn_with_state(verifier, org_auth))
        .with_state(deps)
}

/// Mount the deal module: read all documents + stage config writes + scope-fenced lifecycle
/// verbs. Generic opportunity mutation is not mounted. **Prefer this over
/// `DealModule::all_crud_routes()` for any real deployment.**
///
/// The composing service builds one [`OrgVerifier`] from its JWT secret and passes it here;
/// the surface derives its session from the token, so no tenant crosses the wire in a body.
/// Mount inside the tenant router with the `backbone_orm::PgPool` request extension attached —
/// `org_auth` resolves the scope against that pool.
pub fn create_guarded_deal_routes(
    m: &DealModule,
    pool: PgPool,
    verifier: OrgVerifier,
    selling: Arc<dyn SellingPort>,
    sink: Arc<dyn DealEventSink>,
) -> Router {
    let write = Arc::new(DealWriteService::new(pool));
    Router::new()
        .merge(m.readonly_routes())
        // Stages are tenant config masters: generic (scope-fenced) writes, the recruitment
        // posture for its stage config. Opportunities keep NO generic writes — their lifecycle
        // fields are computed and move only through the verbs below.
        .merge(create_stage_write_routes(m.stage_service.clone()))
        .merge(create_deal_verb_routes(write, selling, sink, verifier))
}
