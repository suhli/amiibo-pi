# 蓝牙运行手册

## 配置与接管

`/etc/amiibo-pi/bluetooth.env` 的 `AMIIBO_ADAPTER` 默认为 hci0，可指定另一个实际存在的 hciN。`AMIIBO_HOST` 可选，用于将配对和重连限制在一个 Switch 的蓝牙地址。修改后重启 amiibo-bt 和 amiibo-pi。设置页面展示 helper 启动时读到的配置。

默认 `AMIIBO_ISOLATE=true`。首次配对或重连时，helper 在调整适配器前保存其地址、别名、设备类别、电源、可发现性、可配对性及两个超时值。它要求原有 bluetooth.service 正在运行。随后在 /run 中添加本项目专有的 systemd drop-in，启动临时 bluetoothd，禁用 input 插件，并使用 Pro 手柄 DeviceID 以及私有的 BlueZ 设备数据库。正常系统的配置和绑定数据库继续保存在原路径。设置可发现性后写入设备类别 0x002508。

接管会重启 bluetooth.service，其他蓝牙连接将中断。自动接管适用于只有一个适配器、使用标准 Ubuntu bluetoothd 启动配置的设备；若系统已有定制插件参数或多个适配器，脚本会报错。高级用户已自行隔离 HID 插件并配置 DeviceID 时，可以设置 `AMIIBO_ISOLATE=false`，由系统 BlueZ 管理绑定。此模式不会启用项目私有绑定数据库。项目的安装及恢复脚本只管理蓝牙和 amiibo-pi 服务。

配对时只在短暂窗口注册默认 agent。首次连接由主机发起 SSP；两个监听器使用 LOW，随后检查该主机实际 ACL 上的 AUTH 和 ENCRYPT 再交付通道。重连使用保存的主机地址主动连接 PSM 17、19，要求 MEDIUM 安全级别。helper 从 Linux HCI monitor 过滤适配器、事件类型和实际双通道主机，单独保存在 `/var/lib/amiibo-bt/bonds`，下次接管前将该绑定载入私有 BlueZ 数据库。这也覆盖主机指示不持久化的临时 key。根目录权限为 0700，key 文件为 0600，Web API 不传输 key。

## 正常停止和恢复

```bash
sudo systemctl stop amiibo-pi amiibo-bt
systemctl status bluetooth --no-pager
bluetoothctl show
```

HTTP 服务关闭 Unix 租约后，helper 释放通道、agent 和 SDP，恢复 bluetoothd 和原适配器属性。异常退出时 ExecStopPost 和下一次启动均检查恢复日志。找不到原适配器或恢复失败时会保留日志并报错。不要手动清除恢复日志来跳过失败。可在仓库执行 `sudo bash deploy/recover.sh` 后再 `sudo systemctl start amiibo-bt amiibo-pi`。

## 根据失败位置排查

| 现象 | 在 Pi 上检查 |
|---|---|
| 无适配器或 Classic 未知 | `rfkill list bluetooth`、`ls /sys/class/bluetooth`、`sudo btmgmt info`。查看 supported settings 中的 br/edr，核实固件和 UART 驱动加载情况 |
| PSM 17/19 绑定失败 | 读 helper 错误中的 PSM 和 errno。检查其他模拟器、input 插件和自定义 bluetoothd unit；用 `systemctl cat bluetooth` 核实临时 drop-in |
| Switch 找不到手柄 | 确认在更改握法／顺序页面，配对窗口仍开放。`bluetoothctl show` 查看别名与可发现性，`sudo hciconfig hci0 -a` 检查 Class |
| 连接但初始化超时 | 比较主机发来的设备信息、SPI、模式设置、玩家灯请求。检查两个通道的对端、加密、MTU 与断开原因 |
| 初次成功但重启后失败 | 检查 helper 日志是否有保存该主机绑定的事件；核实 Pi 地址和主机绑定没有变化。主机端删除旧手柄后可重新配对 |
| MCU 已发现但游戏不识别 | 确认游戏、备份及其注册要求。从 0x22/0x21、0x31、0x11/0x02 到两段读响应及结束帧逐步比较 |
| Unsupported | 查看是否发生非零目标 UID 的写准备或写入命令。需要注册／写入的游戏流程尚未实现 |

采集实机证据时先执行以下命令，再在主机上重新操作，完成后用 Ctrl-C 停止。抓包可能包含 Link Key 和用户标签内容，请保留本地，只分享脱敏的报文类型、时序及错误码。

```bash
sudo sh -c 'umask 077; exec btmon -i hci0 -w /var/tmp/amiibo-pi.btmon'
journalctl -b -u amiibo-bt -u amiibo-pi
```
