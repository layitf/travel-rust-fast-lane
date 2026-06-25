use std::time::Duration;

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod cache;
mod proxy;

use config::Config;
use cache::IpCache;

/// FastLane - 智能网络加速工具
#[derive(Parser)]
#[command(name = "fastlane")]
#[command(about = "本地代理 + 智能IP缓存 + 主动测速", long_about = None)]
struct Cli {
    /// 显示配置信息
    #[arg(short, long)]
    show_config: bool,
    
    /// 清空缓存
    #[arg(long)]
    clear_cache: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
    
    let cli = Cli::parse();
    
    // 加载配置
    let config = Config::load()?;
    
    if cli.show_config {
        println!("{:#?}", config);
        return Ok(());
    }
    
    // 创建 IP 缓存
    let cache = IpCache::new(Duration::from_secs(config.cache_ttl_secs));
    
    if cli.clear_cache {
        cache.clear();
        info!("缓存已清空");
        return Ok(());
    }

    // ============ 添加测试数据 ============
    // 手动填充缓存，用于测试
    // 注意：实际 IP 可能变化，可以用 ping 或 nslookup 获取最新 IP
    cache.put("httpbin.org", "39.144.137.220", 50);
    cache.put("github.com", "20.27.177.113", 120);
    // =====================================
    
    info!("FastLane 启动");
    info!("代理地址: {}", config.proxy_address());
    info!("缓存 TTL: {} 秒", config.cache_ttl_secs);
// 配置文件路径 C:\Users\你的用户名\AppData\Roaming\fastlane\config.toml，如果有，不会再新的
    info!("加速域名: {:?}", config.accelerated_domains);
    
    // 启动代理服务器
    if let Err(e) = proxy::run_proxy(config, cache).await {
        tracing::error!("代理服务器错误: {}", e);
    }
    
    Ok(())
}