# 协议依据与实现边界

## 检查的源文件

- **pcble2gamepad**。研究了 `pro-controller.c` 的 agent、SDP、HCI Link Key、配对窗口、低安全级别入站通道和加密重连，以及 `protocol.c` 的设备身份、SPI 校准和按键位图。上游记录的 NS2 按键测试属于其 C 实现。CSR 与 Realtek 对持久化和重连时序有差异，无法据此外推 Pi 板载芯片。其 Pro 路径没有提供本项目所需的完整 NFC 读事务。[蓝牙实现](https://github.com/wozt/pcble2gamepad/blob/97084dbdc6f83d16663b32a0721ec6b713e2bbe6/poc/pro-controller.c) · [报文实现](https://github.com/wozt/pcble2gamepad/blob/97084dbdc6f83d16663b32a0721ec6b713e2bbe6/poc/protocol.c) · [实机记录](https://github.com/wozt/pcble2gamepad/blob/97084dbdc6f83d16663b32a0721ec6b713e2bbe6/docs/classic-pro-poc.md)
- **joycontrol-kb**。检查了 `run_controller_cli.py` 中的 Pro 模式和 `--nfc`、`protocol.py` 的输出分派，以及 `ir_nfc_mcu.py` 的 UID、固定分段和 MCU CRC。该分支依靠可变 action 在连续输入报告之间推进读取，缺少本项目所需的取消、超时和多会话隔离。[CLI](https://github.com/QueenLinuxGlitch/joycontrol-kb/blob/16bae9911b76cc27739354f13b02085533729eaf/run_controller_cli.py) · [协议](https://github.com/QueenLinuxGlitch/joycontrol-kb/blob/16bae9911b76cc27739354f13b02085533729eaf/joycontrol/protocol.py) · [MCU](https://github.com/QueenLinuxGlitch/joycontrol-kb/blob/16bae9911b76cc27739354f13b02085533729eaf/joycontrol/ir_nfc_mcu.py)
- **switch_amiiboo_in_raspberry，amiibo_edits 分支**。检查了 MCU 的电源／配置路径、轮询状态、读写区分、三帧读应答和 NTAG 页布局。该分支指出部分头字段和重发机制仍未解释，其写操作会修改文件。本项目选择只读事务。[MCU](https://github.com/oniyaa/switch_amiiboo_in_raspberry/blob/3e80cb315dab8c2e2daedc80d447836b7d4d85f7/joycontrol/mcu.py) · [标签对象](https://github.com/oniyaa/switch_amiiboo_in_raspberry/blob/3e80cb315dab8c2e2daedc80d447836b7d4d85f7/joycontrol/nfc_tag.py) · [报文笔记](https://github.com/oniyaa/switch_amiiboo_in_raspberry/blob/3e80cb315dab8c2e2daedc80d447836b7d4d85f7/mcu.md)
- **Nintendo Switch Reverse Engineering**。对照蓝牙报文方向、0x3F、0x21、0x30、0x31、SPI 地址及子命令。另检查了其 packet-nxbt.c 解剖器对报告 ID 和小端 SPI 地址的解析。其笔记与实现对部分 MCU ACK 的描述不同，本项目采用上述 amiibo_edits 实现的应答布局。[HID](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/b354f21ae81f7b0d1d060b2b61e66ad0d9bc1756/bluetooth_hid_notes.md) · [子命令](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/b354f21ae81f7b0d1d060b2b61e66ad0d9bc1756/bluetooth_hid_subcommands_notes.md) · [SPI 区域](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/b354f21ae81f7b0d1d060b2b61e66ad0d9bc1756/spi_flash_notes.md)
- **TagMo**。校验页 2、3、0x82、0x83 和 0x84 的取值与其验证函数交叉检查。本项目额外校验两个 UID 校验字节。界面展示备份中的编号，角色名来自用户输入。[TagArray.kt](https://github.com/HiddenRamblings/TagMo/blob/d4e8246c9f087db70f2ad159a831f851127f033c/app/src/main/java/com/hiddenramblings/tagmo/nfctech/TagArray.kt)

## 控制器

主机在 interrupt 通道发送 HIDP DATA/OUTPUT。0xA2、报告 ID、计数器和八个震动字节之后是子命令。响应保留 0xA1 HIDP 前缀；0x21 在偏移 14 放 ACK、15 放命令。0x02 返回本机蓝牙地址和 Pro 类型 3。0x10 回显四字节小端 SPI 地址及长度，最多读取 29 字节。设备提供只读工厂身份、摇杆校准、颜色、参数以及擦除区默认值。震动开关和 IMU 初始化可以应答，运动数据只有静止样本。

连接初始发送 0x3F，主机可切换到 0x30 或 0x31。标准输入以约 60 Hz 发送，报告 timer 以 5 ms 为单位回绕。就绪取决于收到设备信息、标准或 MCU 报告模式、玩家灯命令。进入 NFC 模式前，0x21 的单字节 0x01 探测按上游 Pro 测试向量返回普通 ACK；模式配置采用 MCU ACK 并计算 CRC。未知子命令返回负应答。左／右摇杆保持工厂校准中心。

## 一次只读事务

1. 网页准备标签时固定一份已验证的 540 字节页面和七字节 UID，并设置 60 秒期限。签名附录可保存在库中；本版传输前 540 字节。Arm 自身不发送成功事件。
2. 主机设置 0x31、通过 0x22 恢复 MCU，通过 0x21 配置 NFC 模式。0x11/0x01 请求电源状态。0x11/0x02 下的 0x01 启动标签发现，0x04 取 NFC 状态。发现帧包含 UID（页数据的 0..3 和 4..8）。
3. 读命令 0x06 的目标 UID 字段全零时，返回 0x3A 的第 1 段（245 字节）、第 2 段（295 字节），然后是携带 UID 的 0x2A 结束帧。传输期间的状态查询不会挤掉下一段。非零目标与 0x08 写请求进入 Unsupported，备份保持只读。
4. 每帧 0x31 共 363 字节，MCU 数据始于偏移 50。313 字节 MCU 区的最后一个字节为前 312 字节的 CRC-8，poly 0x07、初值 0、无反射。配置 ACK 也生成 CRC。重放测试验证整个重组负载。
5. 完成状态在结束帧的传输调用成功后产生，随后标签移除。断开、主机取消、用户移除或过期会丢弃待发送分段。游戏的接受结果需在 NS2 屏幕确认。

## 实机仍需回答的问题

资料中的读头含一段含义尚未完全解释的固定数据，当前按 amiibo_edits 的已知序列发送。固件号、发现状态 1/9、连续两段及结束帧能否满足各 NS2 游戏仍需抓包比较。未实现逐段重传协商、新型标签、IR 相机、MCU 固件更新、真实 IMU、写入和所有者注册。另一个差异是 bluer 提供带授权回调的 DisplayYesNo agent，而上游 Classic PoC 使用 NoInputNoOutput。握手或加密失败时保留失败状态和诊断。
