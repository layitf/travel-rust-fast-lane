## debug-1

```bash
# 清理旧的编译缓存
cargo clean

# 编译
cargo build

# 运行
cargo run

# 测试代理服务器是否运行
curl http://127.0.0.1:1080/
# 输出: FastLane Proxy Running

# 健康检查
curl http://127.0.0.1:1080/health
# 输出: OK

# 正常访问  # 部分情况服务繁忙 503，过段时间重试，curl 默认是直连，不会通过任何代理，需要指定 -x 参数
curl http://httpbin.org/ip
# { origin:'xx.xx.x.x'}

# 代理成功，shell 可能不支持 -x,使用命令提示符
curl -x http://127.0.0.1:1080 http://httpbin.org/ip
# 输出: FastLane Proxy Running
```