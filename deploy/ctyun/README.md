# ctyun 中央控制面部署

## 推荐网络拓扑

```text
Internet -> ctyun ELB HTTPS/WSS :443 -> ECS 127.0.0.1/内网 :8080
                                      -> RDS PostgreSQL（仅 VPC 内网）
```

- Mac/Windows 节点只主动访问 `443/WSS`，终端安全组无需开放入站端口。
- ECS 安全组只允许 ELB 到服务端口；SSH 只允许堡垒机、VPN 或固定管理 IP。
- RDS 安全组只允许中央 ECS 访问 `5432`，不要绑定公网地址。
- ELB 使用 HTTPS 监听并启用 WebSocket；空闲超时需大于 90 秒。
- Web 控制台可以接入 WAF。若设备通道未来改用 mTLS，应让设备域名经四层 ELB 直达 Rust TLS，避免在不支持双向认证的 WAF 上终止。

## Beta：独立 ECS 一体化部署

建议至少使用独立的 `4C8G / 100GB SSD` ECS，不与现有业务测试容器混部。

```bash
cd deploy/ctyun
cp .env.example .env
# 编辑 .env，填入三个随机密钥；不要提交 .env
docker compose up -d --build
docker compose ps
curl --fail http://127.0.0.1:8080/health
```

ELB 健康检查使用 `/health`。默认只在 ECS loopback 暴露 `8080`；如果 ELB 通过 ECS 内网 IP 直连，将 `.env` 中的 `AI_RPA_HOST_BIND` 改为 ECS 私网 IP，并通过安全组限制来源为 ELB，禁止公网直接访问 `8080`。

## 正式环境：ctyun RDS PostgreSQL

创建 RDS PostgreSQL 后，通过内网连接串启动：

```bash
docker compose -f docker-compose.rds.yml up -d --build server
```

`DATABASE_URL`、`AI_RPA_ADMIN_TOKEN`、`AI_RPA_DATA_KEY` 应由服务器密钥文件或密钥管理服务注入。中央服务启动时会幂等初始化 Schema。

## 低负载 RC：CI 发布产物 + 原生服务

当目标 ECS 不适合执行 Rust/Node Docker 构建时，使用 GitHub Actions 生成的
`ai-rpa-central-linux-x64` 产物：

1. 将二进制安装到 `/opt/ai-rpa/bin/ai-rpa-server`，Web 产物安装到 `/opt/ai-rpa/ui`；
2. 以 `docker-compose.postgres.yml` 启动仅绑定 loopback 的 PostgreSQL；
3. 将密钥写入权限为 `0600` 的 `/etc/ai-rpa/server.env`；
4. 参考 `ai-rpa-server.service.example` 安装 systemd 服务；
5. 参考 `nginx.conf.example` 配置 HTTPS/WSS 反向代理。

该方式用于测试/RC；正式环境仍应切换到独立 ECS + ctyun RDS，并由 ELB/WAF
承担公网入口。

## 节点配对

1. 使用管理员令牌登录中央 Web 控制台；
2. 打开“设备”，生成 15 分钟有效的一次性配对码；
3. 在目标电脑桌面端“设备”页面输入中央域名和配对码；
4. 中央控制台应在 5 秒内显示设备在线。

也可以使用 CLI：

```bash
ai-rpa connect \
  --server https://your-ai-control.example.com \
  --code ONE_TIME_CODE \
  --alias "开发 Mac"
```

## 上线门禁

- 两台不同网络的 Mac/Windows 均能保持 WSS、断网补传且不重复；
- 错误配对码、过期码、撤销设备和伪造设备 ID 均被拒绝；
- 远程命令仅投递到目标设备/会话，TTL 到期不执行；
- PostgreSQL 备份恢复演练、72 小时长稳、磁盘满和数据库短时不可用测试通过；
- ELB/WAF/域名证书、访问日志脱敏和告警已配置。
