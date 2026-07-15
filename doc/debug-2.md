## debug-1

增加config 配置，支持自动更行缓存

```bash
curl -x http://127.0.0.1:1080 http://httpbin.org/ip
# 访问显示
# GET http://httpbin.org/ip (来自: 127.0.0.1:5215)
# INFO fast_lane::proxy::http: 缓存命中且验证通过: httpbin.org -> 34.203.4.216 (272ms)

```