//! 自动发现本机运行的 Mower 实例。
//!
//! Mower 可能监听随机端口，先枚举监听端口，再探测 `/status`。
//! alpha 的状态接口需要 token；遇到 403 时通过公开的首页识别实例，不发送任何凭据。

use crate::model::MowerStatus;
use std::time::Duration;

/// 探测到的一个 Mower 实例。
#[derive(Debug, Clone)]
pub struct MowerInstance {
    pub port: u16,
    pub status: MowerStatus,
    pub requires_token: bool,
    pub local_id: Option<String>,
    pub local_port: Option<u16>,
    pub name: String,
    /// 下个任务时间（来自 `/status`），用于在列表里区分多个实例。
    pub next_task_time: Option<String>,
}

/// 本地扫描的状态，供配置界面展示。
#[derive(Debug, Clone)]
pub enum ScanState {
    Scanning,
    Done(Vec<MowerInstance>),
}

pub(crate) fn auto_select<'a>(
    config: &crate::config::Config,
    auto_select_single: bool,
    instances: &'a [MowerInstance],
) -> Option<&'a MowerInstance> {
    if let Some(id) = &config.local_instance_id {
        return instances.iter().find(|instance| {
            instance.local_id.as_ref() == Some(id) && instance.local_port.is_some()
        });
    }
    if !auto_select_single {
        return None;
    }
    let mut eligible = instances
        .iter()
        .filter(|instance| instance.local_id.is_some() && instance.local_port.is_some());
    let only = eligible.next()?;
    eligible.next().is_none().then_some(only)
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

/// 扫描本机所有候选端口，返回确认是 Mower 的实例（按端口排序）。
pub async fn scan_local_mowers() -> Vec<MowerInstance> {
    let ports = candidate_ports();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
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
    found.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| right.local_port.is_some().cmp(&left.local_port.is_some()))
    });
    found.dedup_by_key(|instance| instance.port);
    found
}

/// 探测单个端口是否为 Mower。
async fn probe(client: &reqwest::Client, port: u16) -> Option<MowerInstance> {
    if let Ok(snapshot) = crate::api::fetch_local_snapshot(client, port).await {
        return Some(MowerInstance {
            port: snapshot.mower_port,
            status: MowerStatus::from_api(&snapshot.status),
            requires_token: false,
            local_id: Some(snapshot.id),
            local_port: Some(port),
            name: snapshot.name,
            next_task_time: snapshot.next_task_time,
        });
    }
    let url = format!("http://127.0.0.1:{}/status", port);
    let response = client.get(&url).send().await.ok()?;
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        let index = client
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .ok()?;
        if !index.status().is_success()
            || !index
                .text()
                .await
                .ok()?
                .contains("<title>arknights-mower</title>")
        {
            return None;
        }
        return Some(MowerInstance {
            port,
            status: MowerStatus::Unknown,
            requires_token: true,
            local_id: None,
            local_port: None,
            name: String::new(),
            next_task_time: None,
        });
    }
    if !response.status().is_success() {
        return None;
    }
    let probe: StatusProbe = response.json().await.ok()?;
    Some(MowerInstance {
        port,
        status: MowerStatus::from_api(&probe.status),
        requires_token: false,
        local_id: None,
        local_port: None,
        name: String::new(),
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
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn local_instance(id: &str, port: u16) -> super::MowerInstance {
        super::MowerInstance {
            port,
            status: crate::model::MowerStatus::Sleeping,
            requires_token: false,
            local_id: Some(id.to_string()),
            local_port: Some(port + 100),
            name: String::new(),
            next_task_time: None,
        }
    }

    #[test]
    fn automatically_selects_only_a_unique_or_previously_chosen_instance() {
        let mut config = crate::config::Config::default();
        let instances = vec![local_instance("one", 1000), local_instance("two", 2000)];
        assert!(super::auto_select(&config, true, &instances).is_none());
        assert_eq!(
            super::auto_select(&config, true, &instances[..1])
                .unwrap()
                .local_id
                .as_deref(),
            Some("one")
        );
        assert!(super::auto_select(&config, false, &instances[..1]).is_none());
        config.local_instance_id = Some("two".into());
        assert_eq!(
            super::auto_select(&config, false, &instances)
                .unwrap()
                .local_id
                .as_deref(),
            Some("two")
        );
        config.local_instance_id = Some("missing".into());
        assert!(super::auto_select(&config, false, &instances).is_none());
    }

    fn protected_server(index: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (path, status, body) in [
                ("/mowerhud", 404, "Not Found"),
                ("/status", 403, "Forbidden"),
                ("/", 200, index),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 2048];
                while !request.ends_with(b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(request.starts_with(&format!("get {path} ")));
                assert!(!request.lines().any(|line| line.starts_with("token:")));
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        port
    }

    #[tokio::test]
    async fn discovers_local_read_only_hud_service_without_a_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let count = socket.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
            assert!(request.starts_with("get /mowerhud "));
            assert!(!request.lines().any(|line| line.starts_with("token:")));
            let body = r#"{"protocol":"mowerhud/1","id":"test-instance","name":"测试","mower_port":58000,"status":"sleeping","next_task_time":null,"tasks":[]}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let result = super::probe(&reqwest::Client::new(), port).await.unwrap();
        assert_eq!(result.port, 58000);
        assert_eq!(result.status, crate::model::MowerStatus::Sleeping);
        assert!(!result.requires_token);
    }

    #[tokio::test]
    async fn discovers_token_protected_alpha_through_public_index() {
        let port = protected_server("<html><head><title>arknights-mower</title></head></html>");
        let result = super::probe(&reqwest::Client::new(), port).await;
        assert_eq!(result.as_ref().map(|instance| instance.port), Some(port));
    }

    #[tokio::test]
    async fn ignores_unrelated_forbidden_http_service() {
        let port = protected_server("<html><title>Another service</title></html>");
        assert!(super::probe(&reqwest::Client::new(), port).await.is_none());
    }

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
