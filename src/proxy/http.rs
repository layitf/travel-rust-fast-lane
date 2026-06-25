use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::ConnectInfo,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, Uri};
use tracing::{debug, error, info, warn};

use crate::cache::IpCache;
use crate::Config;

#[derive(Clone)]
pub struct ProxyContext {
    pub cache: IpCache,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
}

pub async fn run_proxy(config: Config, cache: IpCache) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.proxy_addr, config.proxy_port).parse()?;

    // 在移动 config 之前，先提取所有需要的值
    let accelerated_domains = config.accelerated_domains.clone();
    let proxy_addr = config.proxy_addr.clone();
    let proxy_port = config.proxy_port;
    let cache_ttl_secs = config.cache_ttl_secs;

    let context = ProxyContext {
        cache,
        config: Arc::new(config), // config 在这里被移动
        http_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    };

    // ============ 添加测试数据 cache 需要mut 或者 clone ============
    // cache.put("httpbin.org", "52.230.231.139", 50);
    // cache.put("github.com", "20.27.177.113", 120);
    // info!("已预填充缓存测试数据");
    // =====================================
    
    // 关键：使用 `into_make_service_with_connect_info` 来获取客户端地址
    let app = Router::new()
        .route("/", get(|| async { "FastLane Proxy Running" }))
        .route("/health", get(|| async { "OK" }))
        .fallback(proxy_handler)
        .with_state(context);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(
        "FastLane 代理服务器启动: http://{}:{}",
        proxy_addr, proxy_port
    );
    info!("加速域名: {:?}", accelerated_domains);
    info!("缓存 TTL: {} 秒", cache_ttl_secs);
    info!("");
    info!(
        "测试命令: curl -x http://{}:{} http://httpbin.org/ip",
        proxy_addr, proxy_port
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// 代理处理器 - 捕获所有代理请求
async fn proxy_handler(
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::extract::State(ctx): axum::extract::State<ProxyContext>,
    body: Bytes,
) -> Response {
    println!("!!! 请求到达 proxy_handler !!!");
    debug!("代理请求: {} {} (来自: {})", method, uri, addr);

    // 1. 提取目标域名和路径
    let (host, path_and_query) = if let Some(host) = uri.host() {
        let path = uri.path();
        let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
        (host.to_string(), format!("{}{}", path, query))
    } else {
        error!("无法解析主机: {}", uri);
        return (StatusCode::BAD_REQUEST, "Invalid URL").into_response();
    };

    // 2. 检查是否需要加速
    let should_accelerate = ctx
        .config
        .accelerated_domains
        .iter()
        .any(|d| host.ends_with(d));

    // 3. 确定目标 IP/域名
    let (target_host, used_cache) = if should_accelerate {
        if let Some(cached_ip) = ctx.cache.get(&host) {
            info!("缓存命中: {} -> {}", host, cached_ip);
            (cached_ip, true)
        } else {
            warn!("缓存未命中: {}，使用原始域名直连", host);
            (host.clone(), false)
        }
    } else {
        (host.clone(), false)
    };

    // 4. 构建目标 URL
    let scheme = "http";
    let target_url = format!("{}://{}{}", scheme, target_host, path_and_query);
    debug!("转发到: {} (使用缓存: {})", target_url, used_cache);

    // 5. 构建 reqwest 请求
    let req_method = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    };

    let mut req_builder = ctx.http_client.request(req_method, &target_url);

    // 6. 复制请求头
    for (name, value) in headers.iter() {
        let name_str = name.as_str();
        if name_str == "proxy-connection" || name_str == "proxy-authorization" {
            continue;
        }
        if let Ok(val) = value.to_str() {
            req_builder = req_builder.header(name_str, val);
        }
    }

    // 7. 如果使用了缓存的 IP，设置正确的 Host 头
    if used_cache {
        req_builder = req_builder.header("Host", &host);
    }

    // 8. 添加代理相关信息
    req_builder = req_builder.header("X-Forwarded-For", addr.ip().to_string());
    req_builder = req_builder.header("X-Forwarded-Proto", "http");

    // 9. 设置请求体
    if !body.is_empty() {
        req_builder = req_builder.body(body.to_vec());
    }

    // 10. 发送请求
    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            let mut response_builder = http::Response::builder().status(status.as_u16());

            // 复制响应头
            for (name, value) in resp.headers().iter() {
                if let Ok(val) = value.to_str() {
                    response_builder = response_builder.header(name.as_str(), val);
                }
            }

            // 读取响应体
            match resp.bytes().await {
                Ok(body_bytes) => {
                    let body = Body::from(body_bytes.to_vec());
                    response_builder
                        .body(body)
                        .unwrap_or_else(|e| {
                            error!("构建响应失败: {}", e);
                            http::Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Body::from("Internal Server Error"))
                                .unwrap()
                        })
                        .into_response()
                }
                Err(e) => {
                    error!("读取响应体失败: {}", e);
                    (StatusCode::BAD_GATEWAY, "Failed to read response").into_response()
                }
            }
        }
        Err(e) => {
            error!("转发请求失败: {} -> {}", target_url, e);
            (StatusCode::BAD_GATEWAY, format!("Proxy error: {}", e)).into_response()
        }
    }
}
