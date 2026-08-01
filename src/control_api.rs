// src/control_api.rs
// 用于再调试运行时通过 http curl 修改代理配置开关

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::error::Error;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;
type BoxError = Box<dyn Error + Send + Sync>;
use http_body_util::BodyExt;

use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct ControlState {
    pub proxy_enabled: Arc<Mutex<bool>>,
}

impl ControlState {
    pub fn new(enabled: bool) -> Self {
        Self {
            proxy_enabled: Arc::new(Mutex::new(enabled)),
        }
    }
}

pub async fn run_control_api(port: u16, state: ControlState) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    info!("控制 API 启动: http://{}", addr);
    
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state_clone = state.clone();
        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                handle_control_request(req, state_clone.clone())
            });
            if let Err(e) = http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                tracing::error!("控制 API 连接错误: {}", e);
            }
        });
    }
}

// src/control_api.rs
async fn handle_control_request(
    req: Request<Incoming>,
    state: ControlState,
) -> Result<Response<Full<Bytes>>, BoxError> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/status") => {
            let enabled = *state.proxy_enabled.lock().await;
            let response_body = json!({
                "proxy_enabled": enabled,
                "port": 1080
            });
            let body = Full::new(Bytes::from(response_body.to_string()));
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(body)
                .map_err(|e| Box::new(e) as BoxError)
        }
        (&Method::POST, "/set") => {
            // 读取请求体
            let whole_body = req.collect().await
                .map_err(|e| Box::new(e) as BoxError)?;
            let body_bytes = whole_body.to_bytes();
            
            if let Ok(json_body) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                if let Some(enabled) = json_body.get("enabled").and_then(|v| v.as_bool()) {
                    let mut guard = state.proxy_enabled.lock().await;
                    *guard = enabled;
                    
                    // 调用 proxy_control 修改系统代理
                    if enabled {
                        let _ = crate::proxy_control::enable_proxy(1080);
                        info!("代理已开启");
                    } else {
                        let _ = crate::proxy_control::disable_proxy();
                        info!("代理已关闭");
                    }
                    
                    let body = Full::new(Bytes::from("OK"));
                    return Response::builder()
                        .status(StatusCode::OK)
                        .body(body)
                        .map_err(|e| Box::new(e) as BoxError);
                }
            }
            
            let body = Full::new(Bytes::from("Invalid request"));
            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(body)
                .map_err(|e| Box::new(e) as BoxError)
        }
        _ => {
            let body = Full::new(Bytes::from("Not Found"));
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(body)
                .map_err(|e| Box::new(e) as BoxError)
        }
    }
}