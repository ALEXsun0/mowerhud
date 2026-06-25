//! 自动发现本机运行的 Mower 实例。
//!
//! Mower 无 token 时监听一个**随机临时端口**（`bind(0)`），所以无法靠固定端口探测；
//! 必须枚举本机正在监听的 TCP 端口，再逐个探测 `/status`（该接口不需要 token）。

use crate::model::MowerStatus;
use std::time::Duration;

/// 探测到的一个 Mower 实例。
#[derive(Debug, Clone)]
pub struct MowerInstance {
    pub port: u16,
    pub status: MowerStatus,
    /// 下个任务时间（来自 `/status`），用于在列表里区分多个实例。
    pub next_task_time: Option<String>,
}

/// 本地扫描的状态，供配置界面展示。
#[derive(Debug, Clone)]
pub enum ScanState {
    Scanning,
    Done(Vec<MowerInstance>),
}

/// `/status` 探测体：要求同时含 `status` 与 `plan_condition`，作为“这是 Mower”的特征，
/// 避免把别的本地 HTTP 服务误判成 Mower。
#[derive(serde::Deserialize)]
struct StatusProbe {
    status: String,
    #[allow(dead_code)]
    plan_condition: Vec<serde_json::Value>,
    #[serde(default)]
    next_task_time: Option<String>,
}

fn parse_status(s: &str) -> MowerStatus {
    match s {
        "stopped" => MowerStatus::Stopped,
        "sleeping" => MowerStatus::Sleeping,
        "working" => MowerStatus::Working,
        _ => MowerStatus::Unknown,
    }
}

/// 扫描本机所有候选端口，返回确认是 Mower 的实例（按端口排序）。
pub async fn scan_local_mowers() -> Vec<MowerInstance> {
    let ports = candidate_ports();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .unwrap_or_default();

    let mut set = tokio::task::JoinSet::new();
    for port in ports {
        let client = client.clone();
        set.spawn(async move { probe(&client, port).await });
    }

    let mut found = Vec::new();
    while let Some(res) = set.join_next().await {
        if let Ok(Some(instance)) = res {
            found.push(instance);
        }
    }
    found.sort_by_key(|i| i.port);
    found
}

/// 探测单个端口是否为 Mower。
async fn probe(client: &reqwest::Client, port: u16) -> Option<MowerInstance> {
    let url = format!("http://127.0.0.1:{}/status", port);
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let probe: StatusProbe = response.json().await.ok()?;
    Some(MowerInstance {
        port,
        status: parse_status(&probe.status),
        next_task_time: probe.next_task_time,
    })
}

/// 候选端口集合：Windows 下枚举本机监听端口；其他平台退化为常见默认端口。
fn candidate_ports() -> Vec<u16> {
    let mut ports = listening_ports();
    // 兜底加入常见默认端口，避免枚举遗漏
    for p in [58000u16, 3289] {
        if !ports.contains(&p) {
            ports.push(p);
        }
    }
    ports
}

/// 枚举本机正在监听的 TCP 端口（仅保留 >=1024 的用户/临时端口，跳过系统端口）。
#[cfg(windows)]
fn listening_ports() -> Vec<u16> {
    use winapi::shared::tcpmib::{MIB_TCP_STATE_LISTEN, MIB_TCPTABLE_OWNER_PID};
    use winapi::shared::ws2def::AF_INET;
    use winapi::um::iphlpapi::GetExtendedTcpTable;

    // TCP_TABLE_OWNER_PID_LISTENER = 3
    const TCP_TABLE_OWNER_PID_LISTENER: u32 = 3;

    let mut ports = Vec::new();
    unsafe {
        let mut size: u32 = 0;
        // 第一次调用取所需缓冲区大小
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if size == 0 {
            return ports;
        }
        // 用 Vec<u32> 而非 Vec<u8> 作缓冲，保证 4 字节对齐（MIB_TCPTABLE_OWNER_PID 含 DWORD）
        let mut buf = vec![0u32; (size as usize).div_ceil(4)];
        let ret = GetExtendedTcpTable(
            buf.as_mut_ptr() as *mut _,
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if ret != 0 {
            return ports;
        }
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), n);
        for row in rows {
            if row.dwState == MIB_TCP_STATE_LISTEN {
                // dwLocalPort 低 16 位为网络字节序端口
                let port = u16::from_be(row.dwLocalPort as u16);
                if port >= 1024 {
                    ports.push(port);
                }
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Linux：解析 /proc/net/tcp 与 tcp6（无需外部依赖）。
#[cfg(target_os = "linux")]
fn listening_ports() -> Vec<u16> {
    let mut ports = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = std::fs::read_to_string(path) {
            ports.extend(parse_proc_net_tcp(&content));
        }
    }
    ports.retain(|&p| p >= 1024);
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// macOS：调用系统自带的 lsof 列出监听套接字（无 /proc，FFI 结构体易错故走文本解析）。
#[cfg(target_os = "macos")]
fn listening_ports() -> Vec<u16> {
    use std::process::Command;
    // GUI 应用 PATH 可能很精简，用绝对路径更稳
    let output = Command::new("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()
        .or_else(|_| {
            Command::new("lsof")
                .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
                .output()
        });
    let mut ports = Vec::new();
    if let Ok(out) = output
        && let Ok(text) = String::from_utf8(out.stdout)
    {
        ports.extend(parse_lsof(&text));
    }
    ports.retain(|&p| p >= 1024);
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// 其他平台：无统一的轻量枚举方式，退化为探测常见端口。
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn listening_ports() -> Vec<u16> {
    vec![58000, 3289, 8000, 8080]
}

/// 解析 /proc/net/tcp[6] 的内容，返回处于 LISTEN（状态 0A）的本地端口。
/// 行格式：`sl local_address rem_address st ...`，local_address 为 `HEXIP:HEXPORT`。
/// 不加 cfg 限制，便于在任意平台跑单测。
#[cfg(any(test, target_os = "linux"))]
fn parse_proc_net_tcp(content: &str) -> Vec<u16> {
    content
        .lines()
        .skip(1) // 跳过表头
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            // cols[1]=local_address, cols[3]=state
            if cols.len() < 4 || cols[3] != "0A" {
                return None;
            }
            let port_hex = cols[1].rsplit(':').next()?;
            u16::from_str_radix(port_hex, 16).ok()
        })
        .collect()
}

/// 解析 `lsof -nP -iTCP -sTCP:LISTEN` 的输出，提取监听端口。
/// NAME 列形如 `127.0.0.1:58000 (LISTEN)` / `*:631 (LISTEN)` / `[::1]:58000 (LISTEN)`。
#[cfg(any(test, target_os = "macos"))]
fn parse_lsof(text: &str) -> Vec<u16> {
    text.lines()
        .filter_map(|line| {
            let before = line.split(" (LISTEN)").next()?;
            if before == line {
                return None; // 该行没有 (LISTEN)
            }
            let addr = before.split_whitespace().last()?; // 末尾的 host:port
            let port_str = addr.rsplit(':').next()?;
            port_str.parse::<u16>().ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn enumerates_listening_ports() {
        let ports = super::listening_ports();
        println!("listening ports (>=1024): {:?}", ports);
        assert!(
            !ports.is_empty(),
            "GetExtendedTcpTable 应至少枚举到一个监听端口"
        );
    }

    #[test]
    fn parses_proc_net_tcp() {
        // 真实 /proc/net/tcp 片段：第 1 行 LISTEN(0A) 端口 0x1538=5432，
        // 第 2 行 ESTABLISHED(01) 应忽略，第 3 行 LISTEN 端口 0x0050=80。
        let sample = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1538 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000 100 0 0 10 0
   1: 0100007F:9C40 0100007F:1538 01 00000000:00000000 00:00000000 00000000  1000        0 23456 1 0000 20 0 0 10 0
   2: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 34567 1 0000 100 0 0 10 0
";
        let mut ports = super::parse_proc_net_tcp(sample);
        ports.sort_unstable();
        assert_eq!(ports, vec![80, 5432]);
    }

    #[test]
    fn parses_lsof() {
        let sample = "\
COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
Python  1234 user    5u  IPv4 0x1234      0t0  TCP 127.0.0.1:58000 (LISTEN)
Python  1234 user    6u  IPv6 0x5678      0t0  TCP [::1]:58000 (LISTEN)
launchd    1 root    7u  IPv4 0x9abc      0t0  TCP *:631 (LISTEN)
Chrome  4321 user    8u  IPv4 0xdef0      0t0  TCP 127.0.0.1:7890 (ESTABLISHED)
";
        let mut ports = super::parse_lsof(sample);
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(ports, vec![631, 58000]);
    }
}
