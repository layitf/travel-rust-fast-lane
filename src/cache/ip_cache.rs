use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// IP 缓存条目
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// 优选 IP 地址
    pub ip: String,
    /// 延迟（毫秒）
    pub latency: u32,
    /// 过期时间
    expires_at: Instant,
}

impl CacheEntry {
    pub fn new(ip: String, latency: u32, ttl: Duration) -> Self {
        Self {
            ip,
            latency,
            expires_at: Instant::now() + ttl,
        }
    }
    
    /// 检查是否已过期
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// IP 缓存管理器
#[derive(Debug, Clone)]
pub struct IpCache {
    inner: Arc<DashMap<String, CacheEntry>>,
    ttl: Duration,
}

impl IpCache {
    /// 创建新的 IP 缓存
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            ttl,
        }
    }
    
    /// 获取域名的优选 IP（如果缓存存在且未过期）
    pub fn get(&self, domain: &str) -> Option<String> {
        if let Some(entry) = self.inner.get(domain) {
            if !entry.is_expired() {
                debug!("缓存命中: {} -> {}", domain, entry.ip);
                return Some(entry.ip.clone());
            } else {
                // 过期，移除
                drop(entry);
                self.inner.remove(domain);
                debug!("缓存过期: {}", domain);
            }
        }
        None
    }
    
    /// 设置域名的优选 IP
    pub fn set(&self, domain: String, ip: String, latency: u32) {
        let entry = CacheEntry::new(ip, latency, self.ttl);
        debug!("缓存更新: {} -> {} ({}ms)", domain, entry.ip, latency);
        self.inner.insert(domain, entry);
    }

    /// 手动填充缓存（用于测试或预加载）
    pub fn put(&self, domain: &str, ip: &str, latency: u32) {
        self.set(domain.to_string(), ip.to_string(), latency);
    }
    
    /// 获取缓存统计信息
    pub fn stats(&self) -> CacheStats {
        let total = self.inner.len();
        let active = self.inner.iter()
            .filter(|entry| !entry.is_expired())
            .count();
        CacheStats { total, active }
    }
    
    /// 清除所有缓存
    pub fn clear(&self) {
        self.inner.clear();
        debug!("缓存已清空");
    }
}

/// 缓存统计信息
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total: usize,
    pub active: usize,
}