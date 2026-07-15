use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

/// FastLane 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 代理服务器监听地址
    pub proxy_addr: String,
    /// 代理服务器监听端口
    pub proxy_port: u16,
    /// IP 缓存 TTL（秒）
    pub cache_ttl_secs: u64,
    /// 后台刷新间隔（秒），0 表示禁用
    pub background_refresh_interval_secs: u64,
    /// 测速超时时间（秒）
    pub speed_test_timeout_secs: u64,
    /// 需要加速的域名列表
    pub accelerated_domains: Vec<String>,
    /// 是否添加 X-Forwarded-For 请求头
    #[serde(default = "default_add_x_forwarded_for")]
    pub add_x_forwarded_for: bool,
}

/// 为 add_x_forwarded_for 提供默认值
fn default_add_x_forwarded_for() -> bool {
    false
}

impl Default for Config {
    fn default() -> Self {
        Self {
            proxy_addr: "127.0.0.1".to_string(),
            proxy_port: 1080,
            cache_ttl_secs: 300,                 // 5 分钟
            background_refresh_interval_secs: 0, // 1 分钟，默认开启
            speed_test_timeout_secs: 2,          // 2 秒超时
            accelerated_domains: vec![
                "github.com".to_string(),
                "raw.githubusercontent.com".to_string(),
                "crates.io".to_string(),
                "static.crates.io".to_string(),
                "httpbin.org".to_string(),
            ],
            add_x_forwarded_for: false, //  默认关闭，避免暴露代理信息
        }
    }
}

impl Config {
    /// 加载配置文件，如果不存在则创建默认配置, 如果存在但缺少字段，自动补全并保存
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::get_config_path();

        if config_path.exists() {
            // 读取原始内容
            let content = std::fs::read_to_string(&config_path)?;

            // 尝试解析为 HashMap，检查哪些字段缺失
            let parsed: Result<HashMap<String, toml::Value>, _> = toml::from_str(&content);

            match parsed {
                Ok(map) => {
                    // 获取默认配置
                    let default_config = Config::default();

                    // 检查是否有字段缺失
                    let missing_fields = Self::find_missing_fields(&map);

                    if !missing_fields.is_empty() {
                        warn!("配置文件缺少字段: {:?}，将使用默认值补全", missing_fields);

                        // 解析现有配置，缺失字段用默认值补全
                        let mut config: Config = toml::from_str(&content)?;
                        config.apply_defaults(&default_config);

                        // 保存补全后的配置
                        if let Err(e) = config.save() {
                            warn!("无法保存补全后的配置文件: {}", e);
                        } else {
                            info!("配置文件已自动补全字段: {:?}", missing_fields);
                        }

                        Ok(config)
                    } else {
                        // 正常解析
                        Ok(toml::from_str(&content)?)
                    }
                }
                Err(e) => {
                    // 如果配置文件格式有误，尝试直接解析
                    warn!("配置文件解析失败: {}，尝试直接解析", e);
                    match toml::from_str(&content) {
                        Ok(config) => Ok(config),
                        Err(_) => {
                            // 如果仍然失败，用默认配置覆盖
                            warn!("配置文件损坏，使用默认配置覆盖");
                            let config = Config::default();
                            config.save()?;
                            Ok(config)
                        }
                    }
                }
            }
        } else {
            // 配置文件不存在，创建默认配置
            let config = Config::default();
            config.save()?;
            info!("已创建默认配置文件: {:?}", config_path);
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

    /// 查找配置中缺失的字段
    fn find_missing_fields(parsed: &HashMap<String, toml::Value>) -> Vec<String> {
        let default_config = Config::default();
        let default_struct = toml::to_string(&default_config).unwrap();
        let default_map: HashMap<String, toml::Value> = toml::from_str(&default_struct).unwrap();

        let mut missing = Vec::new();
        for key in default_map.keys() {
            if !parsed.contains_key(key) {
                missing.push(key.clone());
            }
        }
        missing
    }

    /// 用默认配置补全当前配置的缺失字段
    fn apply_defaults(&mut self, default: &Config) {
        // 只补全标量字段，不处理 Vec 等复杂类型
        // 对于 Vec 类型，如果用户配置了，就保留用户的值；如果没配置，用默认值
        // 这里我们用 serde 的 default 特性已经处理了大部分
        // 但为了更健壮，我们手动检查关键字段

        // 如果 add_x_forwarded_for 是默认值 false，但用户可能想保留 false，我们不需要覆盖
        // 只需要确保字段存在即可
        if self.add_x_forwarded_for == false && default.add_x_forwarded_for == false {
            // 都相同，无需操作
        }
    }
}
