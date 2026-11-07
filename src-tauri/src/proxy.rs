mod api;
pub mod node;
pub(crate) mod core;
#[cfg(test)]
mod node_tests;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    #[serde(default = "system_mode")]
    pub mode: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_proxy: Option<LocalProxy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<node::NodeOptions>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyEndpoint {
    pub host: String,
    pub port: u16,
}
impl ProxyEndpoint {
    pub fn validate(&self)->Result<(),String> {
        if self.port==0||self.host.parse::<IpAddr>().is_err(){return Err("已提取的代理地址无效，请重新启动副本".into());}
        Ok(())
    }
}
/// Optional first hop on this computer or another reachable host (e.g. a LAN router).
/// Profile credentials belong exclusively
/// to the second hop and are never sent to this endpoint.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalProxy {
    pub mode: String,
    pub host: String,
    pub port: u16,
}
impl LocalProxy {
    pub fn validate(&self) -> Result<(), String> {
        if !["http", "https", "socks5"].contains(&self.mode.as_str()) {
            return Err("前置代理协议应为 HTTP、HTTPS 或 SOCKS5".into());
        }
        // Use the same endpoint validation as the second hop. The serialized
        // LocalProxy/preProxy names remain compatible with existing profiles.
        ProxyConfig { mode: self.mode.clone(), host: self.host.clone(), port: self.port,
            ..Default::default() }.validate()
            .map_err(|_| "前置代理请填写有效的 IP 或域名及端口，不要包含协议或路径".into())
    }
}
fn system_mode() -> String {
    "system".into()
}
impl ProxyConfig {
    pub fn system() -> Self {
        Self {
            mode: system_mode(),
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !["direct", "system", "http", "https", "socks5", "vless", "vmess", "hysteria2"].contains(&self.mode.as_str()) {
            return Err("不支持的代理协议".into());
        }
        node::validate_config(self)?;
        if let Some(local) = &self.pre_proxy {
            if !self.custom() { return Err("链式代理需要先配置副本代理".into()); }
            local.validate()?;
        }
        if let Some(url)=&self.api_url {
            if !self.custom(){return Err("API 提取需要先选择代理协议".into());}
            api::validate_url(url)?;
        }
        if self.custom() {
            if self.api_url.is_none() && (self.port == 0
                || self.host.is_empty()
                || self.host.len() > 253
                || self
                    .host
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || "/\\@?#".contains(c))
            ) {
                return Err("请填写有效的代理地址和端口".into());
            }
            if self.api_url.is_none() && self.host.parse::<IpAddr>().is_err()
                && !self.host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return Err("代理地址应为 IP 或域名，不要包含协议或路径".into());
            }
            if self.username.as_bytes().len() > 255
                || self.username.chars().any(|c| c.is_control() || c == ':')
            {
                return Err("代理用户名格式无效".into());
            }
        }
        Ok(())
    }
    pub fn custom(&self) -> bool {
        ["http", "https", "socks5", "vless", "vmess", "hysteria2"].contains(&self.mode.as_str())
    }
    pub fn has_secret(&self)->bool {self.custom()&&(!self.username.is_empty()||node::is_node(&self.mode))}
}
pub struct Upstream {
    pub config: ProxyConfig,
    pub password: String,
}
impl Upstream {
    pub fn validate(&self) -> Result<(), String> {
        self.config.validate()?;
        if self.config.mode == "hysteria2" && self.password.is_empty() { return Err("节点认证信息为空，请重新导入节点链接".into()); }
        if matches!(self.config.mode.as_str(),"vless"|"vmess")&&uuid::Uuid::parse_str(&self.password).is_err(){return Err("节点 UUID 无效，请重新导入节点链接".into());}
        if self.password.as_bytes().len() > 255 || self.password.chars().any(char::is_control) {
            return Err("代理密码格式无效或超过 255 字节".into());
        }
        Ok(())
    }
}
trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}
type Socket = Box<dyn Stream>;
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
fn credentials(up: &Upstream) -> String {
    if up.config.username.is_empty() {
        String::new()
    } else {
        format!(
            "Proxy-Authorization: Basic {}\r\n",
            STANDARD.encode(format!("{}:{}", up.config.username, up.password))
        )
    }
}
async fn header(stream: &mut (impl AsyncRead + Unpin + ?Sized)) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while bytes.len() < 65536 {
        let b = stream.read_u8().await.map_err(|_| "连接在读取响应时中断")?;
        bytes.push(b);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err("代理响应头过大".into())
}
async fn tls_socket(socket: Socket, host: &str) -> Result<Socket, String> {
    let connector = tokio_native_tls::native_tls::TlsConnector::new()
        .map_err(|_| "无法初始化代理 TLS")?;
    let socket = tokio_native_tls::TlsConnector::from(connector)
        .connect(host, socket).await
        .map_err(|_| "代理 TLS 校验失败，请检查证书和代理协议")?;
    Ok(Box::new(socket))
}
async fn upstream_socket(up: &Upstream) -> Result<Socket, String> {
    let socket: Socket = if let Some(local) = &up.config.pre_proxy {
        let connection = async {
            let socket: Socket = Box::new(TcpStream::connect((local.host.as_str(), local.port))
                .await.map_err(|_| "无法连接地址，请检查前置代理是否运行及监听端口")?);
            let socket = if local.mode == "https" { tls_socket(socket, &local.host).await? } else { socket };
            let first = Upstream {
                config: ProxyConfig { mode: local.mode.clone(), host: local.host.clone(), port: local.port, ..Default::default() },
                password: String::new(),
            };
            proxy_tunnel(socket, &first, &up.config.host, up.config.port).await
        };
        connection.await.map_err(|error| format!("前置代理连接失败：{error}"))?
    } else {
        Box::new(TcpStream::connect((up.config.host.as_str(), up.config.port))
            .await.map_err(|_| "无法连接代理地址，请检查节点和端口")?)
    };
    if up.config.mode == "https" { tls_socket(socket, &up.config.host).await } else { Ok(socket) }
}
async fn tunnel(up: &Upstream, host: &str, port: u16) -> Result<Socket, String> {
    proxy_tunnel(upstream_socket(up).await?, up, host, port).await
}
fn socks_io_error(stage: &str, error: std::io::Error) -> String {
    use std::io::ErrorKind;
    // Never include response bytes, an endpoint URI or authentication data.
    let reason = match error.kind() {
        ErrorKind::UnexpectedEof
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::BrokenPipe
        | ErrorKind::NotConnected => "连接已关闭",
        ErrorKind::TimedOut | ErrorKind::WouldBlock => "等待代理响应超时",
        _ => "网络读写出错",
    };
    let code = error
        .raw_os_error()
        .map(|code| format!("（系统错误码 {code}）"))
        .unwrap_or_default();
    format!("SOCKS5 {stage}失败：{reason}{code}；请核对代理协议和端口，并检查服务商连接限制")
}
async fn proxy_tunnel(mut socket: Socket, up: &Upstream, host: &str, port: u16) -> Result<Socket, String> {
    if up.config.mode == "socks5" {
        let auth = !up.config.username.is_empty();
        socket
            .write_all(if auth { &[5, 1, 2] } else { &[5, 1, 0] })
            .await
            .map_err(|error| socks_io_error("握手发送", error))?;
        let mut response = [0; 2];
        socket
            .read_exact(&mut response)
            .await
            .map_err(|error| socks_io_error("握手响应", error))?;
        if response[0] != 5 {
            return Err("代理返回的不是 SOCKS5 响应，请核对代理协议和端口".into());
        }
        if response[1] == 255 {
            return Err("SOCKS5 代理拒绝了认证方式，请核对节点要求的认证配置".into());
        }
        if !auth && response[1] == 2 {
            return Err("SOCKS5 代理要求用户名和密码，请在节点链接中填写认证信息".into());
        }
        if response != [5, if auth { 2 } else { 0 }] {
            return Err("SOCKS5 认证方式不匹配".into());
        }
        if auth {
            let username = up.config.username.as_bytes();
            let password = up.password.as_bytes();
            let mut bytes = vec![1, username.len() as u8];
            bytes.extend(username);
            bytes.push(password.len() as u8);
            bytes.extend(password);
            socket
                .write_all(&bytes)
                .await
                .map_err(|error| socks_io_error("认证发送", error))?;
            socket
                .read_exact(&mut response)
                .await
                .map_err(|error| socks_io_error("认证响应", error))?;
            if response[0] != 1 {
                return Err("SOCKS5 认证响应格式无效，请核对代理协议和端口".into());
            }
            if response[1] != 0 {
                return Err("代理认证失败，请检查用户名和密码".into());
            }
        }
        if host.len() > 255 {
            return Err("目标域名过长".into());
        }
        let mut request = vec![5, 1, 0];
        match host.parse::<IpAddr>() {
            Ok(IpAddr::V4(address)) => {
                request.push(1);
                request.extend(address.octets());
            }
            Ok(IpAddr::V6(address)) => {
                request.push(4);
                request.extend(address.octets());
            }
            Err(_) => {
                request.extend([3, host.len() as u8]);
                request.extend(host.as_bytes());
            }
        }
        request.extend(port.to_be_bytes());
        socket
            .write_all(&request)
            .await
            .map_err(|_| "SOCKS5 请求失败")?;
        let mut reply = [0; 4];
        socket
            .read_exact(&mut reply)
            .await
            .map_err(|_| "SOCKS5 响应失败")?;
        if reply[0] != 5 || reply[1] != 0 {
            return Err("代理无法连接目标网站".into());
        }
        let len = match reply[3] {
            1 => 4,
            4 => 16,
            3 => socket.read_u8().await.map_err(|_| "SOCKS5 响应无效")? as usize,
            _ => return Err("SOCKS5 响应无效".into()),
        };
        let mut rest = vec![0; len + 2];
        socket
            .read_exact(&mut rest)
            .await
            .map_err(|_| "SOCKS5 响应无效")?;
    } else {
        let target = authority(host, port);
        socket
            .write_all(
                format!(
                    "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{}\r\n",
                    credentials(up)
                )
                .as_bytes(),
            )
            .await
            .map_err(|_| "代理隧道请求失败")?;
        let response = header(&mut *socket).await?;
        let text = String::from_utf8_lossy(&response);
        let status = text
            .lines()
            .next()
            .and_then(|s| s.split_whitespace().nth(1));
        match status {
            Some("200") => {}
            Some("407") => return Err("代理认证失败，请检查用户名和密码".into()),
            _ => return Err("代理拒绝连接目标网站".into()),
        }
    }
    Ok(socket)
}
fn target(value: &str, connect: bool) -> Result<(String, u16, String), String> {
    let url = reqwest::Url::parse(&if connect {
        format!("https://{value}")
    } else {
        value.to_owned()
    })
    .map_err(|_| "代理请求地址无效")?;
    if !["http", "https", "ws", "wss"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("不支持的代理请求".into());
    }
    let host = url
        .host_str()
        .ok_or("代理请求缺少域名")?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = url.port_or_known_default().ok_or("代理请求端口无效")?;
    let route = format!(
        "{}{}",
        url.path(),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    Ok((host, port, route))
}
async fn serve(
    mut client: TcpStream,
    up: Arc<Upstream>,
    mut stop: watch::Receiver<bool>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<(), String> {
    let setup = tokio::time::timeout(Duration::from_secs(20), async {
        let bytes = header(&mut client).await?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "代理请求格式无效")?;
        let mut lines = text.split("\r\n");
        let first = lines.next().ok_or("代理请求为空")?;
        let parts: Vec<_> = first.split_whitespace().collect();
        if parts.len() != 3
            || parts[2] != "HTTP/1.1"
            || !parts[0].bytes().all(|b| b.is_ascii_uppercase())
        {
            return Err("代理请求格式无效".into());
        }
        let connect = parts[0] == "CONNECT";
        let (host, port, route) = target(parts[1], connect)?;
        let mut socket = if connect || up.config.mode == "socks5" {
            tunnel(&up, &host, port).await?
        } else {
            upstream_socket(&up).await?
        };
        if connect {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .map_err(|_| "客户端已关闭")?;
        } else {
            let mut request = format!(
                "{} {} HTTP/1.1\r\n",
                parts[0],
                if up.config.mode == "socks5" {
                    route.as_str()
                } else {
                    parts[1]
                }
            );
            let upgrade = text
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("upgrade:"));
            for line in lines.filter(|s| !s.is_empty()) {
                let key = line.split(':').next().unwrap_or("").to_ascii_lowercase();
                if ["proxy-authorization", "proxy-connection"].contains(&key.as_str())
                    || (!upgrade && key == "connection")
                {
                    continue;
                }
                if !line.contains(':') || line.starts_with([' ', '\t']) {
                    return Err("代理请求头无效".into());
                }
                request.push_str(line);
                request.push_str("\r\n");
            }
            if up.config.mode != "socks5" {
                request.push_str(&credentials(&up));
            }
            if !upgrade {
                request.push_str("Connection: close\r\n");
            }
            request.push_str("\r\n");
            socket
                .write_all(request.as_bytes())
                .await
                .map_err(|_| "代理请求转发失败")?;
        }
        Ok::<Socket, String>(socket)
    });
    let setup = async {
        setup
            .await
            .unwrap_or_else(|_| Err("代理连接超时，请检查节点".into()))
    };
    let result = tokio::select! {result=setup=>result, _=stop.changed()=>return Ok(())};
    match result {
        Ok(mut socket) => {
            tokio::select! {_=tokio::io::copy_bidirectional(&mut client,&mut socket)=>{},_=stop.changed()=>{}}
            Ok(())
        }
        Err(error) => {
            if let Ok(mut last) = failure.lock() {
                *last = Some(error.clone());
            }
            let body = format!("{error}。当前副本不会自动切换为直连。");
            let _=client.write_all(format!("HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
            Err(error)
        }
    }
}
pub struct Relay {
    pub port: u16,
    pub endpoint: Option<ProxyEndpoint>,
    stop: watch::Sender<bool>,
    failure: Arc<Mutex<Option<String>>>,
    core: Option<Arc<core::Core>>,
}
impl Relay { pub fn ready(&self)->bool{self.core.as_ref().is_none_or(|core|core.alive())} }
impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
pub async fn start(up: Upstream, port: u16) -> Result<Relay, String> {
    up.validate()?;
    let extracted=up.config.api_url.is_some();
    let up=api::resolve(up).await?;
    let endpoint=extracted.then(||ProxyEndpoint {host:up.config.host.clone(),port:up.config.port});
    let (up,core)=if node::is_node(&up.config.mode){let (up,core)=core::start(up).await?;(up,Some(core))}else{(up,None)};
    if !up.config.custom() {
        return Err("本地转发需要自定义代理".into());
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|_| "本地代理端口被占用，请关闭副本后重试")?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取本地代理端口")?
        .port();
    let up = Arc::new(up);
    let (stop, mut receiver) = watch::channel(false);
    let connections = receiver.clone();
    let failure = Arc::new(Mutex::new(None));
    let shared_failure = failure.clone();
    let relay_core=core.clone();
    tauri::async_runtime::spawn(async move {
        let core=relay_core;
        let permits = Arc::new(tokio::sync::Semaphore::new(128));
        loop {
            tokio::select! {
                _=receiver.changed()=>break,
                incoming=listener.accept()=>{if let Ok((socket,_))=incoming {if core.as_ref().is_some_and(|core|!core.alive()){if let Ok(mut failure)=shared_failure.lock(){*failure=Some("副本代理内核已退出，请重新启动副本".into());}continue;}if let Ok(permit)=permits.clone().try_acquire_owned() {let up=up.clone();let stop=connections.clone();let failure=shared_failure.clone();tauri::async_runtime::spawn(async move{let _permit=permit;let _=serve(socket,up,stop,failure).await;});}}else{break;}}
            }
        }
    });
    Ok(Relay {
        port,
        endpoint,
        stop,
        failure,
        core,
    })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTest {
    pub ip: Option<String>,
    pub country: Option<String>,
    pub latency: u128,
    pub checked_at: String,
    pub connection: String,
    pub target: String,
    #[serde(default)]
    pub clock: Option<crate::engine::Check>,
}
pub async fn test(up: Upstream) -> Result<ProxyTest, String> {
    let mode = up.config.mode.clone();
    let started = std::time::Instant::now();
    let (client, relay) = http_client(up).await?;
    test_with_client(client, relay, mode, started).await
}
pub async fn http_client(up: Upstream) -> Result<(reqwest::Client, Option<Relay>), String> {
    up.validate()?;
    let mode = up.config.mode.clone();
    let relay = if up.config.custom() {
        Some(start(up, 0).await?)
    } else {
        None
    };
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(4));
    if let Some(relay) = &relay {
        builder = builder.no_proxy().proxy(
            reqwest::Proxy::all(format!("http://127.0.0.1:{}", relay.port))
                .map_err(|_| "无法准备代理测试")?,
        );
    } else if mode == "direct" {
        builder = builder.no_proxy();
    }
    let client = builder.build().map_err(|_| "无法准备网络检测")?;
    Ok((client, relay))
}
async fn test_with_client(
    client: reqwest::Client,
    relay: Option<Relay>,
    mode: String,
    started: std::time::Instant,
) -> Result<ProxyTest, String> {
    let (text, clock) = crate::clock_probe::fetch(&client)
        .await
        .map_err(|message| {
            relay
                .as_ref()
                .and_then(|r| r.failure.lock().ok().and_then(|v| v.clone()))
                .unwrap_or(message)
        })?;
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .map(str::to_owned)
    };
    let latency = started.elapsed().as_millis();
    let (page, route) = tokio::join!(
        crate::claude_probe::probe(&client, "https://claude.ai"),
        crate::claude_probe::trace(&client, "https://claude.ai/cdn-cgi/trace")
    );
    let target = format!(
        "claude.ai {} · {}",
        page.label,
        route.unwrap_or_else(|| "同域名出口未取得，需在副本内复检".into())
    );
    Ok(ProxyTest {
        ip: value("ip=").filter(|v| v.parse::<IpAddr>().is_ok()),
        country: value("loc=")
            .filter(|v| v.len() == 2 && v.bytes().all(|c| c.is_ascii_uppercase())),
        latency,
        checked_at: chrono::Utc::now().to_rfc3339(),
        connection: if mode == "system" {
            "列表 IP 来自通用 Cloudflare 检测，可能与 Claude 分流出口不同。此测试采用系统 / 环境代理，仅覆盖 TCP HTTPS；PAC 和 HTTP/3（QUIC）需在副本内复检".into()
        } else {
            "列表 IP 来自通用 Cloudflare 检测，可能与 Claude 分流出口不同。此测试采用副本指定路径，仅覆盖 TCP HTTPS；HTTP/3（QUIC）需在副本内复检".into()
        },
        target,
        clock: Some(clock),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Starts a temporary Chrome profile through a local SOCKS5 test server; no account login"]
    async fn real_socks_browser_preserves_native_clock_without_automation() {
        use crate::{platform, profiles};
        let root = tempfile::tempdir().unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = platform::close_profile(&self.0);
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(tx)));
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let sender = sender.clone();
                tokio::spawn(async move {
                    let mut greeting = [0; 3];
                    if socket.read_exact(&mut greeting).await.is_err() {
                        return;
                    }
                    assert_eq!(greeting, [5, 1, 0]);
                    socket.write_all(&[5, 0]).await.unwrap();
                    let mut head = [0; 4];
                    socket.read_exact(&mut head).await.unwrap();
                    if head != [5, 1, 0, 3] {
                        return;
                    }
                    let len = socket.read_u8().await.unwrap();
                    let mut host = vec![0; len as usize];
                    socket.read_exact(&mut host).await.unwrap();
                    let destination_port = socket.read_u16().await.unwrap();
                    if host != b"nodecloak-qa.invalid" || destination_port != 7777 {
                        let _ = socket.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                        return;
                    }
                    socket
                        .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                        .await
                        .unwrap();
                    let request = String::from_utf8(header(&mut socket).await.unwrap()).unwrap();
                    if request.starts_with("GET /probe?") {
                        if let Some(tx) = sender.lock().unwrap().take() {
                            let _ = tx.send(request.clone());
                        }
                    }
                    let body = if request.starts_with("GET /session ") {
                        "<!doctype html><title>NodeCloak SOCKS5 QA</title><p>Temporary network and clock check</p><script>fetch('/probe?now='+Date.now()+'&webdriver='+navigator.webdriver+'&offset='+new Date().getTimezoneOffset())</script>"
                    } else {
                        "ok"
                    };
                    let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        let mut preferences = profiles::Preferences::default();
        preferences.startup_url = "http://nodecloak-qa.invalid:7777/session".into();
        // Simulate an old saved source submitted by an older frontend.
        preferences.regional.timezone_mode = "custom".into();
        preferences.regional.timezone = "America/Los_Angeles".into();
        let draft = profiles::Draft {
            name: "SOCKS native clock QA".into(),
            browser: "chrome".into(),
            notes: String::new(),
            tags: vec![],
            proxy: ProxyConfig {
                mode: "socks5".into(),
                host: "127.0.0.1".into(),
                port,
                ..Default::default()
            },
            password: None,
            clear_password: false,
            preferences,
        };
        let profile = profiles::create(root.path(), draft, None).unwrap();
        let _cleanup = Cleanup(profiles::path(root.path(), &profile).unwrap());
        let runtime = Arc::new(profiles::Runtime::default());
        let launch_root = root.path().to_owned();
        let launch_id = profile.id.clone();
        let launch_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || {
            crate::profile_commands::launch(&launch_root, &launch_id, "startup", &launch_runtime)
        })
        .await
        .unwrap()
        .unwrap();
        let request = tokio::time::timeout(Duration::from_secs(45), rx)
            .await
            .unwrap()
            .unwrap();
        let target = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = reqwest::Url::parse(&format!("http://nodecloak-qa.invalid{target}")).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["webdriver"], "false");
        assert_eq!(
            query["offset"].parse::<i32>().unwrap(),
            platform::offset_minutes().unwrap()
        );
        assert!(
            (query["now"].parse::<i64>().unwrap() - chrono::Utc::now().timestamp_millis()).abs()
                < 5000
        );
        assert!(!runtime.1.ready(&profile.id));
        platform::close_profile(&profiles::path(root.path(), &profile).unwrap()).unwrap();
        server.abort();
    }
    #[tokio::test]
    async fn socks_uses_binary_addresses_for_literal_ipv4_and_ipv6_targets() {
        for host in ["127.0.0.1", "::1"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let address = host.parse::<IpAddr>().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut greeting = [0; 3];
                socket.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                socket.write_all(&[5, 0]).await.unwrap();
                let mut prefix = [0; 4];
                socket.read_exact(&mut prefix).await.unwrap();
                let expected = match address {
                    IpAddr::V4(a) => {
                        assert_eq!(prefix, [5, 1, 0, 1]);
                        a.octets().to_vec()
                    }
                    IpAddr::V6(a) => {
                        assert_eq!(prefix, [5, 1, 0, 4]);
                        a.octets().to_vec()
                    }
                };
                let mut value = vec![0; expected.len()];
                socket.read_exact(&mut value).await.unwrap();
                assert_eq!(value, expected);
                assert_eq!(socket.read_u16().await.unwrap(), 443);
                socket
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 1])
                    .await
                    .unwrap();
            });
            let mut upstream = up("socks5", port);
            upstream.config.username.clear();
            upstream.password.clear();
            tunnel(&upstream, host, 443).await.unwrap();
            server.await.unwrap();
        }
    }
    #[tokio::test]
    #[ignore = "Starts two real Chrome windows in temporary profiles; run explicitly for release QA"]
    async fn real_browser_profiles_keep_cookies_proxies_and_close_actions_separate() {
        use crate::{platform, profiles};
        let root = tempfile::tempdir().unwrap();
        struct Cleanup(Vec<std::path::PathBuf>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for path in &self.0 {
                    let _ = platform::close_profile(path);
                }
            }
        }
        async fn mock_proxy(
            label: &'static str,
        ) -> (
            u16,
            tokio::sync::oneshot::Receiver<String>,
            tokio::task::JoinHandle<()>,
        ) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (tx, rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let mut sender = Some(tx);
                loop {
                    let (mut s, _) = listener.accept().await.unwrap();
                    let req = match header(&mut s).await {
                        Ok(b) => String::from_utf8_lossy(&b).to_string(),
                        Err(_) => continue,
                    };
                    if !req.starts_with("GET http://127.0.0.1:7777/") {
                        let _=s.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        continue;
                    }
                    assert!(req.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ="));
                    if req.starts_with("GET http://127.0.0.1:7777/probe") {
                        if let Some(tx) = sender.take() {
                            let _ = tx.send(req.clone());
                        }
                    }
                    let body = if req.contains("/session HTTP") {
                        "<!doctype html><title>NodeCloak release QA</title><p>Temporary browser isolation test</p><script>fetch('/probe')</script>"
                    } else {
                        "ok"
                    };
                    let cookie = if req.contains("/session HTTP") {
                        format!("Set-Cookie: session={label}; Path=/; SameSite=Lax\r\n")
                    } else {
                        String::new()
                    };
                    let _=s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
                }
            });
            (port, rx, task)
        }
        let draft = |name: &str| profiles::Draft {
            name: name.into(),
            browser: "chrome".into(),
            notes: String::new(),
            tags: vec![],
            proxy: ProxyConfig::system(),
            password: None,
            clear_password: false,
            preferences: profiles::Preferences::default(),
        };
        let one = profiles::create(root.path(), draft("QA one"), None).unwrap();
        let two = profiles::create(root.path(), draft("QA two"), None).unwrap();
        let path_one = profiles::path(root.path(), &one).unwrap();
        let path_two = profiles::path(root.path(), &two).unwrap();
        let _cleanup = Cleanup(vec![path_one.clone(), path_two.clone()]);
        let (a, rx_a, server_a) = mock_proxy("one").await;
        let (b, rx_b, server_b) = mock_proxy("two").await;
        let relay_a = start(up("http", a), 0).await.unwrap();
        let relay_b = start(up("http", b), 0).await.unwrap();
        platform::spawn_browser_with_proxy(
            "chrome",
            &path_one,
            "http://127.0.0.1:7777/session",
            "http",
            Some(relay_a.port),
        )
        .unwrap();
        platform::spawn_browser_with_proxy(
            "chrome",
            &path_two,
            "http://127.0.0.1:7777/session",
            "http",
            Some(relay_b.port),
        )
        .unwrap();
        let request_a = tokio::time::timeout(Duration::from_secs(45), rx_a)
            .await
            .unwrap()
            .unwrap();
        let request_b = tokio::time::timeout(Duration::from_secs(45), rx_b)
            .await
            .unwrap()
            .unwrap();
        assert!(request_a.contains("session=one"));
        assert!(!request_a.contains("session=two"));
        assert!(request_b.contains("session=two"));
        assert!(!request_b.contains("session=one"));
        assert!(profiles::running(root.path(), &one).unwrap());
        assert!(profiles::running(root.path(), &two).unwrap());
        platform::close_profile(&path_one).unwrap();
        assert!(!profiles::running(root.path(), &one).unwrap());
        assert!(profiles::running(root.path(), &two).unwrap());
        platform::close_profile(&path_two).unwrap();
        assert!(!profiles::running(root.path(), &two).unwrap());
        server_a.abort();
        server_b.abort();
    }
    #[tokio::test]
    async fn https_proxy_rejects_plaintext_before_credentials_are_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut bytes = [0u8; 4096];
            let len = s.read(&mut bytes).await.unwrap();
            assert_eq!(bytes[0], 22);
            assert!(!String::from_utf8_lossy(&bytes[..len]).contains("secret"));
            s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        });
        assert!(upstream_socket(&up("https", port))
            .await
            .err()
            .unwrap()
            .contains("TLS"));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn rejected_authentication_is_explained_and_never_connects_directly() {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_port = destination.local_addr().unwrap().port();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            header(&mut s).await.unwrap();
            s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await
                .unwrap();
        });
        let relay = start(up("http", port), 0).await.unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", relay.port)).await.unwrap();
        client
            .write_all(
                format!("CONNECT 127.0.0.1:{target_port} HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        assert!(header(&mut client)
            .await
            .unwrap()
            .starts_with(b"HTTP/1.1 502"));
        assert!(relay
            .failure
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("认证"));
        assert!(
            tokio::time::timeout(Duration::from_millis(250), destination.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn plain_http_upstream_auth_replaces_client_auth_and_profiles_route_separately() {
        async fn endpoint(label: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let task = tokio::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                let req = String::from_utf8(header(&mut s).await.unwrap()).unwrap();
                assert!(req.starts_with("GET http://example.com/test HTTP/1.1"));
                assert!(req.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ="));
                assert!(!req.contains("client-secret"));
                s.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{label}",
                        label.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            });
            (port, task)
        }
        let (a, task_a) = endpoint("profile-a").await;
        let (b, task_b) = endpoint("profile-b").await;
        let relay_a = start(up("http", a), 0).await.unwrap();
        let relay_b = start(up("http", b), 0).await.unwrap();
        assert_ne!(relay_a.port, relay_b.port);
        async fn request(port: u16) -> String {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET http://example.com/test HTTP/1.1\r\nHost: example.com\r\nProxy-Authorization: client-secret\r\n\r\n").await.unwrap();
            let mut bytes = vec![];
            s.read_to_end(&mut bytes).await.unwrap();
            String::from_utf8(bytes).unwrap()
        }
        let (ra, rb) = tokio::join!(request(relay_a.port), request(relay_b.port));
        assert!(ra.ends_with("profile-a"));
        assert!(rb.ends_with("profile-b"));
        task_a.await.unwrap();
        task_b.await.unwrap();
    }
    #[tokio::test]
    async fn dropping_relay_cancels_a_stalled_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let relay = start(up("http", port), 0).await.unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", relay.port)).await.unwrap();
        client
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let (mut upstream, _) = listener.accept().await.unwrap();
        header(&mut upstream).await.unwrap();
        drop(relay);
        let mut bytes = [0; 1];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), client.read(&mut bytes))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    fn up(mode: &str, port: u16) -> Upstream {
        Upstream {
            config: ProxyConfig {
                mode: mode.into(),
                host: "127.0.0.1".into(),
                port,
                username: "user".into(),
                ..ProxyConfig::default()
            },
            password: "secret".into(),
        }
    }
    #[test]
    fn validates_proxy_and_target_without_accepting_injected_headers() {
        let mut p = up("http", 80);
        assert!(p.validate().is_ok());
        p.config.host = "host\r\nX:evil".into();
        assert!(p.validate().is_err());
        assert!(target("file:///tmp/a", false).is_err());
        assert_eq!(target("[::1]:443", true).unwrap().1, 443);
    }
    #[tokio::test]
    async fn http_authentication_and_tunnel_bytes_work() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(header(&mut socket).await.unwrap()).unwrap();
            assert!(request.starts_with("CONNECT example.com:443"));
            assert!(request.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ="));
            socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            let mut data = [0; 4];
            socket.read_exact(&mut data).await.unwrap();
            socket.write_all(&data).await.unwrap();
        });
        let mut socket = tunnel(&up("http", port), "example.com", 443).await.unwrap();
        socket.write_all(b"ping").await.unwrap();
        let mut data = [0; 4];
        socket.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"ping");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn socks_auth_and_remote_domain_resolution_work() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 2]);
            socket.write_all(&[5, 2]).await.unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), 1);
            let len = socket.read_u8().await.unwrap();
            let mut user = vec![0; len as usize];
            socket.read_exact(&mut user).await.unwrap();
            assert_eq!(user, b"user");
            let len = socket.read_u8().await.unwrap();
            let mut password = vec![0; len as usize];
            socket.read_exact(&mut password).await.unwrap();
            assert_eq!(password, b"secret");
            socket.write_all(&[1, 0]).await.unwrap();
            let mut request = [0; 5];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 3]);
            let mut domain = vec![0; request[4] as usize];
            socket.read_exact(&mut domain).await.unwrap();
            assert_eq!(domain, b"example.com");
            assert_eq!(socket.read_u16().await.unwrap(), 443);
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 1])
                .await
                .unwrap();
        });
        tunnel(&up("socks5", port), "example.com", 443)
            .await
            .unwrap();
        server.await.unwrap();
    }
    #[tokio::test]
    async fn relay_fails_closed_when_upstream_is_down() {
        let relay = start(up("http", 1), 0).await.unwrap();
        let mut socket = TcpStream::connect(("127.0.0.1", relay.port)).await.unwrap();
        socket
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n")
            .await
            .unwrap();
        let reply = header(&mut socket).await.unwrap();
        assert!(reply.starts_with(b"HTTP/1.1 502"));
    }
}

#[cfg(test)]
mod chain_tests;

#[cfg(test)]
mod socks_diagnostics_tests;

#[cfg(test)]
mod api_tests;
