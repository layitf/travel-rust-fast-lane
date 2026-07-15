use std::time::Duration;

use clap::Parser;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cache;
mod config;
mod proxy;
mod speedtest;

use cache::IpCache;
use config::Config;

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

    // clap 的 #[arg(long)] 属性会自动将字段名 reset_config 转换为命令行参数 --reset-config（下划线变横杠）
    /// 重置配置文件为默认值
    #[arg(long)]
    reset_config: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    // 重置配置文件, 命令行强制重置 fast-lane.exe --reset-config 或 ./fast-lane --reset-config
    if cli.reset_config {
        let config = Config::default();
        config.save()?;
        info!("配置文件已重置为默认值");
        return Ok(());
    }

    // 加载配置（自动补全缺失字段）
    let config = Config::load()?;

    if cli.show_config {
        println!("当前配置:");
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
    // cache.put("httpbin.org", "39.144.137.220", 50);
    // cache.put("github.com", "20.27.177.113", 120);
    // =====================================

    info!("FastLane 启动 v{}", env!("CARGO_PKG_VERSION"));
    info!("代理地址: {}", config.proxy_address());
    info!("缓存 TTL: {} 秒", config.cache_ttl_secs);
    // 配置文件路径 C:\Users\user\AppData\Roaming\fastlane\config.toml，如果有，不会再新的
    info!(
        "后台刷新间隔: {} 秒",
        config.background_refresh_interval_secs
    );
    info!("加速域名: {:?}", config.accelerated_domains);
    info!("X-Forwarded-For: {}", config.add_x_forwarded_for);

    // ========== 启动后台刷新任务 ==========
    let refresh_interval = config.background_refresh_interval_secs;
    if refresh_interval > 0 {
        info!("后台主动刷新已启用，间隔: {} 秒", refresh_interval);

        let cache_clone = cache.clone();
        let domains = config.accelerated_domains.clone();
        let timeout_secs = config.speed_test_timeout_secs;
        let interval = Duration::from_secs(refresh_interval);
        let timeout = Duration::from_secs(timeout_secs);

        tokio::spawn(async move {
            let mut timer = tokio::time::interval(interval);

            // 启动后立即执行一次初始刷新，填充缓存
            info!("执行初始缓存刷新...");
            perform_background_refresh(&cache_clone, &domains, timeout).await;

            loop {
                timer.tick().await;
                info!("执行后台缓存刷新...");
                perform_background_refresh(&cache_clone, &domains, timeout).await;
            }
        });
    } else {
        info!("后台主动刷新已禁用 (background_refresh_interval_secs = 0)，仅执行初始刷新");

        // 执行一次初始刷新后，任务自动结束
        let cache_clone = cache.clone();
        let domains = config.accelerated_domains.clone();
        let timeout = Duration::from_secs(config.speed_test_timeout_secs);

        tokio::spawn(async move {
            info!("执行初始缓存刷新...");
            perform_background_refresh(&cache_clone, &domains, timeout).await;
            info!("初始缓存刷新完成，任务结束");
            // 任务执行完毕后，没有 loop，自动关闭
        });
    }

    // ========== 启动代理服务器（主任务） ==========
    // 启动代理服务器
    if let Err(e) = proxy::run_proxy(config, cache).await {
        error!("代理服务器错误: {}", e);
    }

    Ok(())
}

/// 执行后台刷新（提取为独立函数，代码更清晰）
async fn perform_background_refresh(cache: &IpCache, domains: &[String], timeout: Duration) {
    let port = 80;
    for domain in domains {
        // 检查缓存是否过期或即将过期
        // 简单起见，直接刷新（如果缓存有效，find_fastest_ip 会很快返回）
        if let Some((ip, rtt)) = speedtest::find_fastest_ip(domain, port, timeout).await {
            cache.set(domain.clone(), ip.to_string(), rtt.as_millis() as u32);
            info!("后台刷新完成: {} -> {} ({}ms)", domain, ip, rtt.as_millis());
        } else {
            info!("后台刷新失败: {} (暂不可达)", domain);
        }
    }
}
