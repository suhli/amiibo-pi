use crate::state::*;
use amiibo_core::Error;
use axum::{
    body::{to_bytes, Body},
    extract::{
        ws::{Message, WebSocketUpgrade},
        Path, Query, Request as HttpRequest, State,
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;
use tower_http::services::ServeDir;

#[derive(Debug)]
struct Problem(StatusCode, &'static str, String);
impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"code":self.1,"message":self.2}))).into_response()
    }
}
impl From<Error> for Problem {
    fn from(e: Error) -> Self {
        let (status, code) = match e {
            Error::Unsupported(_) => (StatusCode::UNPROCESSABLE_ENTITY, "Unsupported"),
            Error::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "InvalidBackup"),
            Error::Duplicate => (StatusCode::CONFLICT, "Duplicate"),
            Error::Missing => (StatusCode::NOT_FOUND, "NotFound"),
            Error::Sql(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Storage"),
        };
        Self(status, code, e.to_string())
    }
}
type Output = Result<Json<Value>, Problem>;
fn ok() -> Output {
    Ok(Json(json!({"ok":true})))
}
fn authorized(token: &str, provided: Option<&str>) -> bool {
    provided.is_some_and(|v| bool::from(token.as_bytes().ct_eq(v.as_bytes())))
}

async fn auth(State(app): State<Shared>, req: HttpRequest, next: Next) -> Response {
    let headers = req.headers();
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let websocket = if req.uri().path().ends_with("/events") {
        headers
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').find_map(|p| p.trim().strip_prefix("auth.")))
    } else {
        None
    };
    if !authorized(&app.token, bearer.or(websocket)) {
        return Problem(
            StatusCode::UNAUTHORIZED,
            "Unauthorized",
            "Valid access token required".into(),
        )
        .into_response();
    }
    let mut res = next.run(req).await;
    if (res.status().is_client_error() || res.status().is_server_error())
        && !res
            .headers()
            .get("content-type")
            .is_some_and(|v| v == "application/json")
    {
        let status = res.status();
        res = Problem(
            status,
            "InvalidRequest",
            status.canonical_reason().unwrap_or("Request failed").into(),
        )
        .into_response();
    }
    res.headers_mut().insert(
        "cache-control",
        axum::http::HeaderValue::from_static("no-store"),
    );
    res
}

pub fn router(app: Shared, web: String) -> Router {
    let api=Router::new()
        .route("/health",get(||async{Json(json!({"ok":true}))}))
        .route("/system",get(|State(a):State<Shared>|async move{Json(a.diagnostics.clone())}))
        .route("/bluetooth/adapters",get(|State(a):State<Shared>|async move{Json(a.diagnostics["adapters"].clone())}))
        .route("/snapshot",get(|State(a):State<Shared>|async move{a.snapshot().map(Json).map_err(Problem::from)}))
        .route("/controller/status",get(status)).route("/nfc/status",get(status))
        .route("/controller/{command}",post(controller)).route("/nfc/{command}",post(nfc))
        .route("/simulation/replay",post(|State(a):State<Shared>|async move{dispatch(&a,Action::Replay).await}))
        .route("/amiibo",get(list).post(upload)).route("/amiibo/{id}",delete(remove).patch(edit)).route("/amiibo/{id}/select",post(select))
        .route("/history",get(|State(a):State<Shared>|async move{Ok::<_,Problem>(Json(a.db.lock().unwrap().history()?))}))
        .route("/logs",get(|State(a):State<Shared>|async move{Json(a.logs.lock().unwrap().clone())}))
        .route("/settings/debug",post(debug)).route("/events",get(events))
        .fallback(||async{not_found()}).layer(middleware::from_fn_with_state(app.clone(),auth));
    Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new(web))
        .with_state(app)
}
async fn status(State(a): State<Shared>) -> Json<Status> {
    Json(a.status.lock().unwrap().clone())
}
async fn dispatch(a: &App, action: Action) -> Output {
    let (tx, rx) = oneshot::channel();
    a.tx.try_send(Request {
        action,
        response: tx,
    })
    .map_err(|_| {
        Problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "Busy",
            "Controller queue unavailable".into(),
        )
    })?;
    rx.await
        .map_err(|_| {
            Problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Offline",
                "Controller stopped".into(),
            )
        })?
        .map_err(|s| Problem(StatusCode::CONFLICT, "InvalidState", s))?;
    ok()
}
#[derive(Deserialize)]
struct Button {
    button: Option<String>,
}
async fn controller(
    State(a): State<Shared>,
    Path(op): Path<String>,
    Query(q): Query<Button>,
) -> Output {
    let action = match op.as_str() {
        "pair" => Action::Pair,
        "disconnect" => Action::Disconnect,
        "reconnect" => Action::Reconnect,
        "press" => Action::Press(q.button.unwrap_or_default()),
        _ => return Err(not_found()),
    };
    dispatch(&a, action).await
}
async fn nfc(State(a): State<Shared>, Path(op): Path<String>) -> Output {
    if op == "arm"
        && a.status
            .lock()
            .unwrap()
            .mtu
            .is_some_and(|size| size < switch_pro::NFC_REPORT_LEN)
    {
        return Err(Problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Unsupported",
            "NFC requires a 363-byte interrupt MTU".into(),
        ));
    }
    let action = match op.as_str() {
        "arm" => Action::Arm,
        "cancel" => Action::Cancel,
        _ => return Err(not_found()),
    };
    dispatch(&a, action).await
}
fn not_found() -> Problem {
    Problem(
        StatusCode::NOT_FOUND,
        "NotFound",
        "Unknown operation".into(),
    )
}
async fn list(State(a): State<Shared>) -> Output {
    Ok(Json(json!(a.db.lock().unwrap().list()?)))
}
#[derive(Deserialize)]
struct Upload {
    name: String,
}
async fn upload(
    State(a): State<Shared>,
    q: Result<Query<Upload>, axum::extract::rejection::QueryRejection>,
    body: Body,
) -> Output {
    let Query(q) = q.map_err(|_| {
        Problem(
            StatusCode::BAD_REQUEST,
            "InvalidName",
            "Provide a name query parameter".into(),
        )
    })?;
    let bytes = to_bytes(body, 572).await.map_err(|_| {
        Problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "TooLarge",
            "Maximum upload is 572 bytes".into(),
        )
    })?;
    let id = a.db.lock().unwrap().add(&q.name, &bytes)?;
    a.changed();
    Ok(Json(json!({"id":id})))
}
#[derive(Deserialize)]
struct Edit {
    category: String,
    favorite: bool,
}
async fn edit(
    State(a): State<Shared>,
    Path(id): Path<i64>,
    body: Result<Json<Edit>, axum::extract::rejection::JsonRejection>,
) -> Output {
    let Json(e) = body.map_err(|_| {
        Problem(
            StatusCode::BAD_REQUEST,
            "InvalidMetadata",
            "Expected category and favorite".into(),
        )
    })?;
    a.db.lock().unwrap().edit(id, &e.category, e.favorite)?;
    a.changed();
    ok()
}
async fn remove(State(a): State<Shared>, Path(id): Path<i64>) -> Output {
    a.db.lock().unwrap().delete(id)?;
    a.changed();
    ok()
}
async fn select(State(a): State<Shared>, Path(id): Path<i64>) -> Output {
    a.db.lock().unwrap().select(id)?;
    a.changed();
    ok()
}
#[derive(Deserialize)]
struct DebugQuery {
    enabled: bool,
}
async fn debug(State(a): State<Shared>, Query(q): Query<DebugQuery>) -> Output {
    a.debug
        .store(q.enabled, std::sync::atomic::Ordering::Relaxed);
    a.changed();
    ok()
}
async fn events(State(a): State<Shared>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let host = headers.get("host").and_then(|v| v.to_str().ok());
        if origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
            != host
        {
            return Problem(
                StatusCode::FORBIDDEN,
                "Origin",
                "WebSocket origin does not match the host".into(),
            )
            .into_response();
        }
    }
    ws.max_frame_size(4096).max_message_size(4096).protocols(["amiibo-v1"]).on_upgrade(move|mut socket|async move{
        let mut version=a.version.subscribe();
        loop{
            if a.closing.load(std::sync::atomic::Ordering::Relaxed){break;}
            let data=match a.snapshot(){Ok(s)=>s.to_string(),Err(e)=>{a.log(e.to_string());break;}};
            if !matches!(tokio::time::timeout(std::time::Duration::from_secs(5),socket.send(Message::Text(data.into()))).await,Ok(Ok(()))){break;}
            tokio::select!{
                r=version.changed()=>if r.is_err(){break;},
                msg=socket.recv()=>if !matches!(msg,Some(Ok(Message::Ping(_)|Message::Pong(_)))){break;}
            }
        }
    })
}
