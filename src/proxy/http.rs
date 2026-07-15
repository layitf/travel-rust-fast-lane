use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

use crate::cache::IpCache;
use crate::speedtest;
use crate::Config;

#[derive(Clone)]
pub struct ProxyContext {
    pub cache: IpCache,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
}

// 自定义错误类型
#[derive(Debug)]
pub struct ProxyError {
    status: StatusCode,
    message: String,
}

impl ProxyError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for ProxyError {}

/// 启动 Hyper HTTP 代理服务器
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

    let listener = TcpListener::bind(addr).await?;
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

    loop {
        let (stream, remote_addr) = listener.accept().await?;
        let context_clone = context.clone();

        tokio::task::spawn(async move {
            let io = TokioIo::new(stream);

            // 使用 service_fn 并显式指定错误类型
            let service = service_fn(move |req: Request<Incoming>| {
                handle_request(req, context_clone.clone(), remote_addr)
            });

            // 使用 serve_connection 并处理结果
            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                error!("连接处理错误 [{}]: {}", remote_addr, e);
            }
        });
    }
}

/// 处理单个请求 - 返回 Result 使用 Box<dyn std::error::Error>
async fn handle_request(
    req: Request<Incoming>,
    ctx: ProxyContext,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
    debug!(
        "收到请求: {} {} (来自: {})",
        req.method(),
        req.uri(),
        client_addr
    );

    // ============ 处理 CONNECT 方法（HTTPS 隧道） ============
    if req.method() == hyper::Method::CONNECT {
        return handle_connect(req, ctx, client_addr).await;
    }

    // ============ 处理普通 HTTP 请求 ============
    handle_http(req, ctx, client_addr).await
}

/// 处理 CONNECT 请求（HTTPS 隧道）
async fn handle_connect(
    req: Request<Incoming>,
    ctx: ProxyContext,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
    // 解析目标：CONNECT github.com:443
    let authority = req
        .uri()
        .authority()
        .ok_or_else(|| ProxyError::new(StatusCode::BAD_REQUEST, "Missing authority"))?;
    let authority_str = authority.as_str();

    let (host, port_str) = match authority_str.split_once(':') {
        Some((h, p)) => (h, p),
        None => (authority_str, "443"),
    };

    let port: u16 = port_str
        .parse()
        .map_err(|_| ProxyError::new(StatusCode::BAD_REQUEST, "Invalid port"))?;

    info!("CONNECT 请求: {}:{} (来自: {})", host, port, client_addr);

    // ============ 判断是否需要加速 ============
    let should_accelerate = ctx
        .config
        .accelerated_domains
        .iter()
        .any(|d| host.ends_with(d));

    let target_addr = if should_accelerate {
        // 需要加速：从缓存获取 IP
        info!("CONNECT 加速: {}", host);

        if let Some(entry) = ctx.cache.get_entry(host) {
            // 验证 IP 是否可达
            let timeout = Duration::from_secs(2);
            match tokio::time::timeout(
                timeout,
                TcpStream::connect(format!("{}:{}", entry.ip, port)),
            )
            .await
            {
                Ok(Ok(_)) => {
                    info!("CONNECT 缓存命中: {} -> {}:{}", host, entry.ip, port);
                    format!("{}:{}", entry.ip, port)
                }
                _ => {
                    warn!("CONNECT 缓存 IP 不可达，重新测速: {} -> {}", host, entry.ip);
                    ctx.cache.remove(host);
                    match speedtest::find_fastest_ip(host, port, Duration::from_secs(2)).await {
                        Some((ip, rtt)) => {
                            ctx.cache
                                .set(host.to_string(), ip.to_string(), rtt.as_millis() as u32);
                            info!(
                                "CONNECT 重新测速完成: {} -> {}:{} ({}ms)",
                                host,
                                ip,
                                port,
                                rtt.as_millis()
                            );
                            format!("{}:{}", ip, port)
                        }
                        None => {
                            warn!("CONNECT 测速失败，使用直连: {}:{}", host, port);
                            format!("{}:{}", host, port)
                        }
                    }
                }
            }
        } else {
            // 缓存未命中，触发测速
            info!("CONNECT 缓存未命中，开始测速: {}", host);
            match speedtest::find_fastest_ip(host, port, Duration::from_secs(2)).await {
                Some((ip, rtt)) => {
                    ctx.cache
                        .set(host.to_string(), ip.to_string(), rtt.as_millis() as u32);
                    info!(
                        "CONNECT 测速完成: {} -> {}:{} ({}ms)",
                        host,
                        ip,
                        port,
                        rtt.as_millis()
                    );
                    format!("{}:{}", ip, port)
                }
                None => {
                    warn!("CONNECT 测速失败，使用直连: {}:{}", host, port);
                    format!("{}:{}", host, port)
                }
            }
        }
    } else {
        // ============ 不在加速列表中：直连（纯转发） ============
        debug!("CONNECT 直连（不在加速列表）: {}:{}", host, port);
        format!("{}:{}", host, port)
    };

    // ============ 建立到目标服务器的连接 ============
    let mut target_stream = TcpStream::connect(&target_addr).await.map_err(|e| {
        ProxyError::new(StatusCode::BAD_GATEWAY, format!("Connection failed: {}", e))
    })?;

    // ============ 获取客户端的底层流并建立双向隧道 ============
    // 由于 hyper 的限制，我们需要通过升级机制获取原始流
    // 这里使用 tokio::io::copy_bidirectional 进行双向转发

    info!("CONNECT 隧道建立: {} -> {}", host, target_addr);

    // TODO: 实现双向隧道转发
    // 注意：这里我们需要在响应发送后继续转发数据
    // 但 hyper 的 serve_connection 会处理这个
    // 我们需要使用 hyper 的 upgrade 机制

    // 由于 upgrade 需要更复杂的处理，我们暂时返回 200
    // 实际隧道转发需要 hyper 的 upgrade 支持
    // 目前只返回 200 Connection Established
    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::new()))
        .map_err(|e| {
            ProxyError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response error: {}", e),
            )
        })?;

    warn!("CONNECT 隧道暂未实现数据转发（仅返回 200）");
    Ok(response)
}

/// 处理普通 HTTP 请求
async fn handle_http(
    req: Request<Incoming>,
    ctx: ProxyContext,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
    let uri = req.uri().clone();
    let host = uri.host().unwrap_or("");
    let method = req.method().clone();

    // 1. 提取路径
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let path_and_query = if path_and_query.is_empty() {
        "/"
    } else {
        path_and_query
    };

    // 2. 检查是否需要加速
    let should_accelerate = ctx
        .config
        .accelerated_domains
        .iter()
        .any(|d| host.ends_with(d));

    // 3. 确定目标 IP/域名
    let (target_host, used_cache) = if should_accelerate {
        if let Some(entry) = ctx.cache.get_entry(host) {
            let port = 80;
            let timeout = Duration::from_secs(2);
            let ip = entry.ip.clone();

            match tokio::time::timeout(timeout, TcpStream::connect(format!("{}:{}", ip, port)))
                .await
            {
                Ok(Ok(_)) => {
                    info!("缓存命中: {} -> {} ({}ms)", host, ip, entry.latency);
                    (ip, true)
                }
                _ => {
                    warn!("缓存 IP 不可达，移除并重新测速: {} -> {}", host, ip);
                    ctx.cache.remove(host);
                    info!("开始重新测速: {}", host);
                    if let Some((new_ip, rtt)) =
                        speedtest::find_fastest_ip(host, 80, Duration::from_secs(2)).await
                    {
                        ctx.cache
                            .set(host.to_string(), new_ip.to_string(), rtt.as_millis() as u32);
                        info!(
                            "重新测速完成: {} -> {} ({}ms)",
                            host,
                            new_ip,
                            rtt.as_millis()
                        );
                        (new_ip.to_string(), true)
                    } else {
                        warn!("重新测速失败，使用原始域名直连: {}", host);
                        (host.to_string(), false)
                    }
                }
            }
        } else {
            info!("缓存未命中，开始测速: {}", host);
            if let Some((ip, rtt)) =
                speedtest::find_fastest_ip(host, 80, Duration::from_secs(2)).await
            {
                ctx.cache
                    .set(host.to_string(), ip.to_string(), rtt.as_millis() as u32);
                info!("测速完成: {} -> {} ({}ms)", host, ip, rtt.as_millis());
                (ip.to_string(), true)
            } else {
                warn!("测速失败，使用原始域名直连: {}", host);
                (host.to_string(), false)
            }
        }
    } else {
        (host.to_string(), false)
    };

    // 4. 构建目标 URL
    let scheme = uri.scheme_str().unwrap_or("http");
    let target_url = format!("{}://{}{}", scheme, target_host, path_and_query);
    debug!("转发到: {}", target_url);

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
    for (name, value) in req.headers().iter() {
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
        req_builder = req_builder.header("Host", host);
    }

    // 8. 添加代理相关信息
    if ctx.config.add_x_forwarded_for {
        req_builder = req_builder.header("X-Forwarded-For", client_addr.ip().to_string());
        req_builder = req_builder.header("X-Forwarded-Proto", "http");
    }

    // 9. 读取请求体
    let body_bytes = req
        .collect()
        .await
        .map_err(|e| ProxyError::new(StatusCode::BAD_REQUEST, format!("Body error: {}", e)))?
        .to_bytes();

    if !body_bytes.is_empty() {
        req_builder = req_builder.body(body_bytes.to_vec());
    }

    // 10. 发送请求
    let resp = req_builder
        .send()
        .await
        .map_err(|e| ProxyError::new(StatusCode::BAD_GATEWAY, format!("Request error: {}", e)))?;

    let status = resp.status();
    let mut response_builder = http::Response::builder().status(status.as_u16());

    // 复制响应头
    for (name, value) in resp.headers().iter() {
        if let Ok(val) = value.to_str() {
            response_builder = response_builder.header(name.as_str(), val);
        }
    }

    // 读取响应体
    let body_bytes = resp.bytes().await.map_err(|e| {
        ProxyError::new(
            StatusCode::BAD_GATEWAY,
            format!("Response body error: {}", e),
        )
    })?;

    let body = Full::new(Bytes::from(body_bytes.to_vec()));
    let response = response_builder.body(body).map_err(|e| {
        ProxyError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Response error: {}", e),
        )
    })?;

    Ok(response)
}
