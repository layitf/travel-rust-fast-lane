use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use dns_lookup::lookup_host;
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

/// 对域名进行测速，返回最快的可达 IP
pub async fn find_fastest_ip(
    domain: &str,
    port: u16,
    timeout: Duration,
) -> Option<(IpAddr, Duration)> {
    let ips = resolve_domain(domain).await;
    if ips.is_empty() {
        warn!("域名 {} 无法解析到任何 IP", domain);
        return None;
    }

    debug!("域名 {} 解析到 {} 个 IP", domain, ips.len());

    let mut tasks = Vec::new();
    for ip in ips {
        let task = tokio::spawn(async move {
            let rtt = measure_rtt(ip, port, timeout).await;
            (ip, rtt)
        });
        tasks.push(task);
    }

    let mut best_ip = None;
    let mut best_rtt = Duration::from_secs(u64::MAX);

    for task in tasks {
        if let Ok((ip, Some(rtt))) = task.await {
            if rtt < best_rtt {
                best_rtt = rtt;
                best_ip = Some(ip);
            }
        }
    }

    if let Some(ip) = best_ip {
        info!("测速完成: {} -> {} (RTT: {}ms)", domain, ip, best_rtt.as_millis());
        Some((ip, best_rtt))
    } else {
        warn!("{} 所有 IP 均不可达", domain);
        None
    }
}

/// 使用 dns-lookup 解析域名
async fn resolve_domain(domain: &str) -> Vec<IpAddr> {
    // domain 是 &str，可能被释放，先转换为 String，获得所有权
    let domain_owned = domain.to_string();
    
    // dns-lookup 的 lookup_host 会阻塞，所以用 tokio::task::spawn_blocking
    match tokio::task::spawn_blocking(move || lookup_host(&domain_owned)).await {
        Ok(Ok(ips)) => {
            // 只保留 IPv4
            ips.into_iter().filter(|ip| ip.is_ipv4()).collect()
        }
        Ok(Err(e)) => {
            error!("DNS 查询失败 {}: {}", domain, e);
            vec![]
        }
        Err(e) => {
            error!("DNS 查询任务失败: {}", e);
            vec![]
        }
    }
}

// 备用 resolve_domain 实现
// use std::net::ToSocketAddrs;

// async fn resolve_domain(domain: &str) -> Vec<IpAddr> {
//     let domain_owned = domain.to_string();
    
//     match tokio::task::spawn_blocking(move || {
//         // 使用标准库的 ToSocketAddrs
//         let addrs: Vec<SocketAddr> = (domain_owned.as_str(), 80)
//             .to_socket_addrs()
//             .ok()
//             .into_iter()
//             .flatten()
//             .collect();
//         addrs.into_iter().map(|addr| addr.ip()).filter(|ip| ip.is_ipv4()).collect::<Vec<_>>()
//     }).await {
//         Ok(ips) => ips,
//         Err(e) => {
//             error!("DNS 查询任务失败: {}", e);
//             vec![]
//         }
//     }
// }

/// 测量到目标 IP:PORT 的 TCP 连接延迟
async fn measure_rtt(ip: IpAddr, port: u16, timeout: Duration) -> Option<Duration> {
    let addr = SocketAddr::new(ip, port);
    let start = Instant::now();

    match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(_)) => Some(start.elapsed()),
        _ => None,
    }
}
