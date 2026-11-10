# 测试与复现

## 工作区检查

在 Linux 安装 README 中的编译依赖，使用支持当前架构的 Rust 工具链。以下命令在仓库根目录执行。

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --release

pnpm --dir web install --frozen-lockfile
pnpm --dir web run typecheck
pnpm --dir web run check
pnpm --dir web run build
node tests/simulation.mjs target/release/amiibo-pi
```

测试主机和标签由测试生成。没有使用商业 amiibo 内容、NFC 私钥或 Nintendo 游戏。单元测试覆盖身份和 SPI 应答、按键释放、MCU 电源、发现、轮询、两段读和结束帧、已知 CRC 测试向量、未知／短命令、取消和超时。Linux 专用测试通过 AF_UNIX 的 SOCK_SEQPACKET 替身验证 SCM_RIGHTS 交接、数据包边界、EOF 和缺失 fd；HCI monitor 测试检查只接受目标适配器和主机的 Link Key 事件。另有测试覆盖低 MTU 保留普通按键、拒绝 0x31 模式，以及第一段传输后移除和更换标签。

端到端脚本创建临时数据库、随机令牌及随机 localhost 端口，启动正式 Axum 二进制的 Simulation 模式，并通过 WebSocket 等待事件。它验证未授权请求、无效／重复／超大备份、选择、收藏、分类、读响应、取消、四轮断开／重连和历史。最后停止服务并清理测试目录。

## 本次验证环境

已在 Windows 及 WSL Ubuntu 22.04 x86_64 使用 Rust 1.97.1 检查。Linux 工作区执行了严格 Clippy、测试和 Release 构建。另以 Ubuntu 22.04 目标库和 GCC 11 交叉构建 armv7-unknown-linux-gnueabihf 与 aarch64-unknown-linux-gnu。两个目标的 10 项 Rust 测试均在 QEMU 下通过，两种架构的 Release 服务均通过同一份 HTTP/WebSocket 端到端脚本。网页使用 Node 24 与 pnpm 11，在 390 px 和 1024 px 宽度检查了五个页面，并通过实际文件输入、令牌登录、主机模拟与扫描结果更新走通流程。TypeScript 与 Svelte 检查无错误或警告。

`pnpm --registry=https://registry.npmjs.org --dir web audit` 对锁文件的检查剩余一项低危构建依赖 deepmerge，来自 Svelte Vite 插件。此次查询未提供已修复版本；生产 Pi 提供编译后的静态资产。详情见 [GHSA-gcqx-w48m-w2h4](https://github.com/advisories/GHSA-gcqx-w48m-w2h4)。

部署脚本通过 Bash 语法检查；systemd-analyze 在暂存的 Ubuntu 22.04 BlueZ 5.64 和项目 unit 上通过了静态校验，ExecStart 路径指向暂存的实际可执行文件。WSL 未连接 Pi 的蓝牙适配器或 Switch 2。实际配对、游戏识别、Pi 板上原生运行、掉电恢复和 systemd 开机验收按[兼容性矩阵](compatibility.md)执行。
