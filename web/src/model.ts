export type Link =
  | "Idle"
  | "Preparing"
  | "Pairing"
  | "Connected"
  | "Initialized"
  | "Ready"
  | "Disconnected"
  | "Reconnecting"
  | "Error";
export type Scan =
  | "Idle"
  | "Armed"
  | "Discovered"
  | "Reading"
  | "Transferred"
  | "Cancelled"
  | "TimedOut"
  | "Unsupported"
  | "Failed";
export interface Status {
  link: Link;
  peer: string | null;
  report: number | null;
  mtu: number | null;
  power: number;
  scan: Scan;
  armed_name: string | null;
  error: string | null;
  simulation: boolean;
}
export interface Tag {
  id: number;
  name: string;
  uid: string;
  figure: string;
  category: string;
  favorite: boolean;
  selected: boolean;
}
export interface Snapshot {
  controller: Status;
  library: Tag[];
  history: { time: number; name: string; outcome: Scan; simulation: boolean }[];
  logs: { time: number; message: string }[];
  system: {
    os: string;
    arch: string;
    adapter: string;
    allowed_host?: string | null;
    adapters: string[];
    classic: boolean | null;
    bluez: string;
    service: string;
    management: string;
  };
  bind: string;
  debug: boolean;
}
export const links: Record<Link, string> = {
  Idle: "空闲",
  Preparing: "准备适配器",
  Pairing: "等待配对",
  Connected: "已连接",
  Initialized: "正在初始化",
  Ready: "手柄就绪",
  Disconnected: "已断开",
  Reconnecting: "正在重连",
  Error: "连接错误",
};
export const scans: Record<Scan, string> = {
  Idle: "未准备",
  Armed: "等待游戏读取",
  Discovered: "主机已发现标签",
  Reading: "正在传输",
  Transferred: "读响应已发送",
  Cancelled: "已取消",
  TimedOut: "等待超时",
  Unsupported: "不支持此操作",
  Failed: "传输失败",
};
export const stamp = (v: number) =>
  new Date(v * 1000).toLocaleString("zh-CN", { hour12: false });
