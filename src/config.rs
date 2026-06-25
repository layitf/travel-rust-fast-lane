use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// FastLane 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 代理服务器监听地址
    pub proxy_addr: String,
    /// 代理服务器监听端口
    pub proxy_port: u16,
    /// IP 缓存 TTL（秒）
    pub cache_ttl_secs: u64,
    /// 后台测速间隔（秒）
    pub speed_test_interval_secs: u64,
    /// 需要加速的域名列表
    pub accelerated_domains: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            proxy_addr: "127.0.0.1".to_string(),
            proxy_port: 1080,
            cache_ttl_secs: 300,      // 5 分钟
            speed_test_interval_secs: 60, // 1 分钟
            accelerated_domains: vec![
                "github.com".to_string(),
                "raw.githubusercontent.com".to_string(),
                "crates.io".to_string(),
                "static.crates.io".to_string(),
                "httpbin.org".to_string(),
            ],
        }
    }
}

impl Config {
    /// 加载配置文件，如果不存在则创建默认配置
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::get_config_path();
        
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            let config = Config::default();
            config.save()?;
            Ok(config)
        }
    }
    
    /// 保存配置文件
    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::get_config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }
    
    /// 获取配置文件路径
    fn get_config_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("fastlane");
        config_dir.join("config.toml")
    }
    
    /// 获取代理服务器完整地址
    pub fn proxy_address(&self) -> String {
        format!("{}:{}", self.proxy_addr, self.proxy_port)
    }
}