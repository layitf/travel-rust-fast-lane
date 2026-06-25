

## 📦 Installation

```bash
# 运行
cargo run -p fast-lane

# 添加依赖
cargo add fast-lane

# 构建
cargo build -p fast-lane


```

## used

```rs
use fast_lane::some_module;

extern crate "fast-lane" as fast_lane;
```

如何选择合适的 background_refresh_interval_secs 值？

场景	                cache_ttl_secs	    background_refresh_interval_secs	效果
追求极致速度，网络稳定	 600 秒（10 分钟）	  120 秒（2 分钟）	                  缓存长期有效，后台提前刷新
追求实时性，IP 变化频繁	 120 秒（2 分钟）	    30 秒                              缓存更新快，几乎总是最新的
节省资源，网络质量差	   1800 秒（30 分钟）	  300 秒（5 分钟）                    减少测速次数，降低网络开销

