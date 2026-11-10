# amiibo-pi

amiibo-pi 把树莓派的内置蓝牙适配器用作第一代 Pro Controller，通过 Bluetooth Classic HID 携带按键和 NFC/IR MCU 报告。手机或电脑可以导入个人 amiibo 备份，选择标签，并等待 Switch 游戏发起读取。运行时是两个 Rust 进程和一组静态网页。

**当前状态**　Ubuntu x86_64、armhf 和 arm64 Release 均已构建。两种 ARM 产物在 QEMU 中通过协议测试与网页 API 端到端模拟。Raspberry Pi 3B+ 板载蓝牙与未破解 Switch 2 的发现、配对、按键和游戏读取仍待实机验收。上游的 NS2 手柄验证使用的是其他适配器。完整的逐项标准在[实机验收矩阵](docs/compatibility.md)。

## 用预编译包安装

先在 Pi 运行 `dpkg --print-architecture`。将对应的 `amiibo-pi-0.1.0-armhf.tar.gz` 或 `amiibo-pi-0.1.0-arm64.tar.gz` 及同名 `.sha256` 校验文件复制到 Pi。在该目录执行以下命令。这些包使用 Ubuntu 22.04 的目标库构建，内含两个 Rust 程序、网页静态资源、源码和服务脚本。

```bash
pkg="amiibo-pi-0.1.0-$(dpkg --print-architecture).tar.gz"
sha256sum -c "$pkg.sha256"
tar -xzf "$pkg"
cd amiibo-pi
sudo apt install bluez libdbus-1-3 openssl
sudo ./target/release/amiibo-bt diagnose
sudo bash deploy/install.sh
sudo cat /etc/amiibo-pi/server.env
```

通过 SSH 转发或局域网登录的步骤见下方。构建自定义版本时使用以下源码流程。

## 从源码安装

先保留 SSH 连接，记录系统、用户空间位数和蓝牙能力。目标是 Pi 3B+ 的 Ubuntu，不预设系统为 arm64。蓝牙接管期间当前适配器上的其他蓝牙设备会断开。

```bash
uname -a
getconf LONG_BIT
dpkg --print-architecture
cat /etc/os-release
sudo apt update
sudo apt install build-essential pkg-config libdbus-1-dev bluez openssl
bluetoothctl --version
systemctl is-active bluetooth
sudo btmgmt info
```

构建使用 Rust 1.97.1、C 编译器和系统 D-Bus 开发包。网页构建使用 Node.js 24 和 pnpm 11.25.0。SQLite 随 Rust 依赖编译。请先安装当前架构的 rustup、Node 和 pnpm；在已有工具链上运行以下命令。

```bash
rustup toolchain install 1.97.1 --profile minimal
export RUSTUP_TOOLCHAIN=1.97.1
node --version
pnpm --version
bash deploy/build.sh
sudo ./target/release/amiibo-bt diagnose
sudo bash deploy/install.sh
systemctl status amiibo-bt amiibo-pi --no-pager
```

Pi 上默认单任务编译以减少内存占用。32 位系统或内存紧张时，可在另一台电脑运行 `pnpm --dir web install --frozen-lockfile` 和 `pnpm --dir web run build`，把生成的 `web/dist/` 整个目录复制到 Pi 的同一工作树，然后在 Pi 上执行 `AMIIBO_PREBUILT_WEB=1 bash deploy/build.sh`。安装正式服务后无需 Node。生成便携安装包时，在包含已提交源码和网页构建依赖的 Linux 工作树中执行 `AMIIBO_RELEASE_DIR=/path/to/release bash deploy/package.sh armhf` 或将末尾架构改为 `arm64`。输出及 SHA-256 位于 `target/packages/`。

安装脚本创建服务账号、受限配置文件、随机访问令牌，并启用两个 systemd unit。重复安装保留已有配置。默认地址是 `127.0.0.1:8787`。可以在电脑上通过 SSH 转发打开网页。

```bash
# 在 Pi 本地查看令牌
sudo cat /etc/amiibo-pi/server.env
# 在电脑上执行，替换成实际用户名和地址
ssh -N -L 8787:127.0.0.1:8787 user@pi-address
```

在浏览器打开 [本机控制台](http://127.0.0.1:8787)，输入 `AMIIBO_TOKEN`。手机访问时，将 `/etc/amiibo-pi/server.env` 中的 `AMIIBO_BIND` 改为 Pi 的受信局域网 IP 和端口，执行 `sudo systemctl restart amiibo-pi`。HTTP 与 WebSocket 都需要令牌。跨越不受信网络时使用 SSH 或带 TLS 的反向代理；HTTP 会以明文传输令牌。

## 首次配对与读取

1. 打开 Switch 2 的「手柄 → 更改握法／顺序」，在网页「手柄」页点击「开始配对」。首次配对窗口持续最多 180 秒。
2. 在「诊断」查看状态。两个 PSM 必须来自同一主机，helper 检查 ACL 的认证、加密和通道 MTU。普通报告需要 50 字节，NFC 需要 363 字节。只有收到设备信息请求、报告模式和玩家灯命令后，页面才显示手柄就绪。
3. 在主机按键测试界面，用网页发送 A、B 或 L+R 并确认屏幕反馈。
4. 在「标签库」导入自己合法获得的备份，可分类、搜索和收藏。支持 540 字节 NTAG215 页面数据和带 32 字节附加签名的 572 字节备份。校验 UID、BCC、锁定位、容量容器与配置页。
5. 在「概览」选择标签，点击「准备标签」，并在 60 秒内让游戏发起扫描。「读响应已发送」表示系统已成功发送两段标签数据及结束报告。**游戏显示预期角色**才满足实机读取的验收标准。
6. 再次准备可换用另一个标签。写入、所有者注册和需要这些操作的游戏流程返回 `Unsupported`。程序不含 dump、私有密钥或解密材料。

## 无硬件开发

```bash
pnpm --dir web install --frozen-lockfile
pnpm --dir web run build
export AMIIBO_TOKEN="$(openssl rand -hex 32)"
cargo run --locked -p amiibo-pi -- --simulate
```

用同一 shell 中的令牌登录，点击配对、准备标签，然后点击「模拟游戏读取」。页面、主机地址和历史均标记 Simulation。模拟主机通过相同的协议状态机读取数据。默认不启用模拟，Linux 正式服务使用 helper。自动化复现见[测试说明](docs/testing.md)。

## 运行与故障排查

```bash
sudo /usr/local/libexec/amiibo-pi/amiibo-bt diagnose
journalctl -b -u amiibo-bt -u amiibo-pi --no-pager
sudo btmgmt --index 0 info
sudo hciconfig hci0 -a
sudo bash deploy/recover.sh
```

最后一条用于停止两个项目服务并恢复原蓝牙环境。正常使用时无需执行。诊断中的 `classic: null` 表示管理能力查询不可用。PSM 被占用、配对超时、主机删除了旧绑定及异常退出的处理见[蓝牙运行手册](docs/bluetooth.md)。实机发生错误时记录 Pi 架构、BlueZ、内核、NS2 系统版本、游戏和报文里程碑。

## 实现与来源

- [架构与 API](docs/architecture.md) 说明权限边界、持久化和事件模型。
- [协议研究笔记](docs/protocol.md) 记录源码版本、实际采用的行为及尚待验证的假设。
- 本项目使用 [GPL-3.0-only](LICENSE)。SDP 记录及部分校准常量来自 pcble2gamepad 的 Classic PoC，保留 [NUXBT](licenses/NUXBT-LICENSE) 与 [NXBT](licenses/NXBT-LICENSE) 的 MIT 声明。NFC 报文依据 joycontrol 两个 GPL 分支的实现和公开分析重新编写。
- Rust 与网页依赖均使用锁文件。逐项版本及包声明的许可证见[依赖清单](licenses/dependencies.tsv)。Ubuntu 的 BlueZ 和 D-Bus 由发行版提供，实际版本可用 `dpkg-query -W bluez libdbus-1-3` 检查。
