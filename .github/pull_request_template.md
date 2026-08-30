## 变更说明

<!-- 说明问题、方案和用户可见变化。 -->

## 验证

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `npm --prefix apps/desktop test`
- [ ] `npm --prefix apps/desktop run build`

验证平台：

- [ ] macOS
- [ ] Windows Native
- [ ] Windows WSL
- [ ] 未执行的平台已明确标注 `NOT RUN`

## 安全与隐私

- [ ] 未提交密钥、Webhook、签名材料、真实提示词、聊天内容、企业源码或个人路径
- [ ] 新增网络入口、命令执行或凭据处理已说明威胁和授权边界
- [ ] UI 或日志样例已经脱敏

## 兼容性

<!-- 说明 Hook 配置、数据库、CLI 或 API 是否存在兼容性变化。 -->
