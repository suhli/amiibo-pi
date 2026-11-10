# 服务与接口

**amiibo-core** 校验导入内容，并使用 SQLite 事务管理标签、收藏、分类、选择及最近 1000 次扫描结果。数据库只接收绑定参数和 BLOB，用户文件名只作为显示文本。删除当前标签会清空选择，已经放置的标签拥有独立副本。

**switch-pro** 只依赖数据模型和时钟输入。Host 报文驱动子命令、只读 SPI 和 MCU 状态机。MockTransport 重放主机初始化、校验输入报告、重组两段数据。每条新连接创建一个 Controller；取消、断开、超时均终止当前事务。

**amiibo-bt** 独占一次适配器租约。Linux helper 注册 BlueZ Agent 和 SDP，使用 bluer 的 BR/EDR SeqPacket 绑定 17/19。通过组权限 0660 的 Unix socket，用 SCM_RIGHTS 将两个已连接的 fd 交给 Web 服务。IPC 仅接受配对或重连两个操作码。客户端关闭 socket 后 helper 关闭会话并还原属性。Link Key 只进入 root 的私有存储。

**amiibo-pi** 以普通系统账号运行 Axum、SQLite 和唯一的 controller actor。它只持有已经连接的通道，不能打开原始 HCI socket。actor 按顺序处理主机包、定时报告和至多 16 个排队操作。报告定时器跳过错过的 tick。状态变化驱动 watch 通知，WebSocket 合并过期快照。事件日志保留 160 条；debug 每 600 个输入报告记录一次计数，并以每秒至多一条的频率采样主机命令类型、长度和负应答。静态资源由 Axum 提供。

## 状态来源

Idle → Preparing → Pairing（或 Reconnecting）→ Connected → Initialized → Ready。成功取得加密的双通道进入 Connected，实际主机初始化推进后续状态。显式断开进入 Disconnected。IO 或初始化超时进入 Error 并记录原因。已建立的硬件连接异常断开后，最多自动重试五次，退避 1、2、4、8、16 秒，可取消。重启后在启用自动恢复且存在已保存主机时尝试重连。

Scan 有独立的 Idle、Armed、Discovered、Reading、Transferred、Cancelled、TimedOut、Unsupported、Failed。模拟与实机共用这套状态和历史记录，额外保留 simulation 来源。选择一个标签与把它放到 MCU 是两个操作。

## API

所有 `/api` 请求需 `Authorization: Bearer TOKEN`。WebSocket 浏览器客户端连接 `/api/events`，请求子协议 `amiibo-v1`、`auth.TOKEN`；服务端只协商 `amiibo-v1`。令牌不进入 URL。浏览器 Origin 必须匹配 Host。错误返回 `{"code":"…","message":"…"}` 及相应 HTTP 状态码。

| 方法 | 路径（省略 /api） | 内容 |
|---|---|---|
| GET | /health、/system、/bluetooth/adapters | 服务状态及启动时的诊断 |
| GET | /snapshot、/events | 完整快照、实时 WebSocket |
| GET | /controller/status、/nfc/status | 同一会话的连接与 MCU 状态 |
| POST | /controller/pair、/controller/reconnect、/controller/disconnect | 启动或取消连接操作；以实时状态判断结果 |
| POST | /controller/press?button=A | 约 180 ms 按键；支持 A、B、X、Y、Plus、L+R |
| GET、POST | /amiibo | 列出标签；上传原始字节并附查询参数 name |
| PATCH、DELETE | /amiibo/{id} | 设置 category、favorite，或删除 |
| POST | /amiibo/{id}/select | 选择下次准备的标签 |
| POST | /nfc/arm、/nfc/cancel | 放置所选标签、移除标签 |
| POST | /simulation/replay | 仅模拟模式，要求已准备标签 |
| GET | /history、/logs | 最近 100 次结果、最近事件 |
| POST | /settings/debug?enabled=true | 切换本次运行的采样日志 |

上传示例（TOKEN 取自本地配置）。

```bash
curl -H "Authorization: Bearer $TOKEN" --data-binary @my-backup.bin \
  'http://127.0.0.1:8787/api/amiibo?name=MyTag'
```

网页按名称、UID、编号和分类在当前快照中筛选。适配器接管及主机限制属于 root 配置；设置页展示有效配置并支持即时调整日志级别。
