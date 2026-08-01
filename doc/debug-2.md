## debug-1

增加config 配置，支持自动更行缓存

```bash
curl -x http://127.0.0.1:1080 http://httpbin.org/ip
# 访问显示
# GET http://httpbin.org/ip (来自: 127.0.0.1:5215)
# INFO fast_lane::proxy::http: 缓存命中且验证通过: httpbin.org -> 34.203.4.216 (272ms)

# 设置日志级别
set RUST_LOG=debug cargo run -- --unset-proxy

# 从 显式配置的透明代理 更新为 项目内启用/关闭代理

# 自动设置代理（传递给 cargo run）
cargo run -- -a

# 取消代理设置（传递给 cargo run）
cargo run -- --unset-proxy

# 运行中改变配置项
# 开启代理
curl -X POST http://127.0.0.1:1081/set -H "Content-Type: application/json" -d "{\"enabled\":true}"

# 关闭代理
curl -X POST http://127.0.0.1:1081/set -H "Content-Type: application/json" -d "{\"enabled\":false}"

# 查看状态
curl http://127.0.0.1:1081/status
```

热加载配置，更新配置是重新加载
监听配置文件变化，当用户修改配置时自动应用

```rust
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

fn watch_config(config_path: PathBuf) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    
    watcher.watch(&config_path, RecursiveMode::NonRecursive)?;
    
    for event in rx {
        match event {
            Ok(notify::Event { kind: notify::EventKind::Modify(_), .. }) => {
                info!("配置文件已修改，重新加载...");
                let new_config = Config::load().unwrap();
                // 应用新配置（包括代理开关）
                if new_config.auto_proxy {
                    let _ = proxy_control::enable_proxy(new_config.proxy_port);
                } else {
                    let _ = proxy_control::disable_proxy();
                }
            }
            _ => {}
        }
    }
}
```