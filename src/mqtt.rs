//! MQTT 5.0 over WebSocket 通用客户端 (对应 Python 端 `utils/mqtt.py`).
//!
//! 仅实现手机端二维码登录所需的子集: CONNECT / CONNACK / SUBSCRIBE / SUBACK /
//! PUBLISH / PINGREQ / PINGRESP / DISCONNECT, 支持用户属性与服务器重定向.

use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::{QmError, Result};

/// MQTT 属性 ID 枚举.
#[allow(dead_code)]
pub mod property_id {
    pub const SERVER_KEEP_ALIVE: u8 = 0x13;
    pub const SERVER_REFERENCE: u8 = 0x1C;
    pub const REASON_STRING: u8 = 0x1F;
    pub const AUTH_METHOD: u8 = 0x15;
    pub const USER_PROPERTY: u8 = 0x26;
}

/// MQTT 消息.
#[derive(Debug, Clone)]
pub struct MqttMessage {
    #[allow(dead_code)]
    pub topic: String,
    pub payload: Vec<u8>,
    /// 消息中的用户属性 (例如 QQ 推送的 `type`).
    pub properties: HashMap<String, String>,
}

impl MqttMessage {
    /// 将 payload 解析为 JSON, 失败时返回 `None`.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.payload).ok()
    }
}

/// MQTT 属性集合.
#[derive(Debug, Clone, Default)]
pub struct MqttProperties {
    pub user_property: Vec<(String, String)>,
    pub auth_method: Option<String>,
    pub server_keep_alive: Option<u16>,
    pub server_reference: Option<String>,
    pub reason_string: Option<String>,
}

impl MqttProperties {
    pub fn user_property(mut self, pairs: &[(&str, &str)]) -> Self {
        self.user_property = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self
    }

    pub fn auth_method(mut self, m: &str) -> Self {
        self.auth_method = Some(m.to_string());
        self
    }
}

/// MQTT 5.0 over WebSocket 客户端.
pub struct MqttClient {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    #[allow(dead_code)]
    pub host: String,
    #[allow(dead_code)]
    pub port: u16,
    #[allow(dead_code)]
    pub path: String,
    pub keep_alive: u16,
    packet_id: AtomicU16,
    recv_buf: Vec<u8>,
}

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
    out
}

fn encode_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.extend((bytes.len() as u16).to_be_bytes());
    out.extend(bytes);
    out
}

fn encode_properties(props: &MqttProperties) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(m) = &props.auth_method {
        body.push(property_id::AUTH_METHOD);
        body.extend(encode_string(m));
    }
    for (k, v) in &props.user_property {
        body.push(property_id::USER_PROPERTY);
        body.extend(encode_string(k));
        body.extend(encode_string(v));
    }
    let mut out = encode_varint(body.len() as u64);
    out.extend(body);
    out
}

fn build_connect(client_id: &str, keep_alive: u16, props: &MqttProperties) -> Vec<u8> {
    let mut var = Vec::new();
    var.extend(encode_string("MQTT"));
    var.push(5); // MQTT 5.0
    var.push(0x02); // clean start
    var.extend(keep_alive.to_be_bytes());
    var.extend(encode_properties(props));
    var.extend(encode_string(client_id));
    let mut packet = vec![0x10];
    packet.extend(encode_varint(var.len() as u64));
    packet.extend(var);
    packet
}

fn build_subscribe(packet_id: u16, topic: &str, props: &MqttProperties) -> Vec<u8> {
    let mut var = Vec::new();
    var.extend(packet_id.to_be_bytes());
    var.extend(encode_properties(props));
    var.extend(encode_string(topic));
    var.push(0x00); // QoS 0
    let mut packet = vec![0x82];
    packet.extend(encode_varint(var.len() as u64));
    packet.extend(var);
    packet
}

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut multiplier: u64 = 1;
    let mut value: u64 = 0;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        value += ((byte & 0x7F) as u64) * multiplier;
        if byte & 0x80 == 0 {
            break;
        }
        multiplier *= 128;
    }
    Some(value)
}

fn read_string(data: &[u8], pos: &mut usize) -> Option<String> {
    let len = u16::from_be_bytes([*data.get(*pos)?, *data.get(*pos + 1)?]) as usize;
    *pos += 2;
    let s = data.get(*pos..*pos + len)?;
    *pos += len;
    String::from_utf8(s.to_vec()).ok()
}

fn parse_properties(data: &[u8], pos: &mut usize) -> Option<MqttProperties> {
    let plen = read_varint(data, pos)? as usize;
    let end = (*pos).saturating_add(plen);
    let mut props = MqttProperties::default();
    while *pos < end {
        let pid = read_varint(data, pos)? as u8;
        match pid {
            property_id::SERVER_KEEP_ALIVE => {
                props.server_keep_alive = Some(u16::from_be_bytes([*data.get(*pos)?, *data.get(*pos + 1)?]));
                *pos += 2;
            }
            property_id::SERVER_REFERENCE => {
                props.server_reference = read_string(data, pos);
            }
            property_id::REASON_STRING => {
                props.reason_string = read_string(data, pos);
            }
            property_id::AUTH_METHOD => {
                props.auth_method = read_string(data, pos);
            }
            property_id::USER_PROPERTY => {
                let k = read_string(data, pos)?;
                let v = read_string(data, pos)?;
                props.user_property.push((k, v));
            }
            _ => {
                // 遇到未知属性停止解析 (当前服务器仅使用上述属性).
                break;
            }
        }
    }
    Some(props)
}

impl MqttClient {
    /// 建立连接并完成 MQTT CONNECT / CONNACK 握手 (自动跟随服务器重定向).
    ///
    /// Args:
    ///     host / port / path: WebSocket 地址.
    ///     client_id: MQTT 客户端标识.
    ///     keep_alive: 心跳间隔 (秒).
    ///     properties: CONNECT 属性 (auth method / user property).
    ///     headers: WebSocket 握手附加请求头.
    pub async fn connect(
        host: &str,
        port: u16,
        path: &str,
        client_id: &str,
        keep_alive: u16,
        properties: &MqttProperties,
        headers: &[(String, String)],
    ) -> Result<Self> {
        let mut current_path = path.to_string();
        let mut redirect_count = 0;
        loop {
            let url = format!("wss://{host}:{port}{current_path}");
            let mut request = url.into_client_request().map_err(|e| QmError::Network(e.to_string()))?;
            let hdrs = request.headers_mut();
            hdrs.insert("Sec-WebSocket-Protocol", HeaderValue::from_static("mqtt"));
            for (k, v) in headers {
                if let (Ok(name), Ok(value)) = (
                    tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(k.as_bytes()),
                    HeaderValue::from_str(v),
                ) {
                    hdrs.insert(name, value);
                }
            }
            let (ws, _resp) = tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| QmError::Network(format!("MQTT WebSocket 握手失败: {e}")))?;

            let mut client = MqttClient {
                ws,
                host: host.to_string(),
                port,
                path: current_path.clone(),
                keep_alive,
                packet_id: AtomicU16::new(1),
                recv_buf: Vec::new(),
            };

            let connect_packet = build_connect(client_id, keep_alive, properties);
            client.send_raw(&connect_packet).await?;

            match client.wait_connack().await? {
                ConnackOutcome::Accepted { keep_alive } => {
                    if let Some(ka) = keep_alive {
                        client.keep_alive = ka;
                    }
                    return Ok(client);
                }
                ConnackOutcome::Redirect(server_reference) => {
                    redirect_count += 1;
                    if redirect_count > 5 {
                        return Err(QmError::Network("MQTT 重定向次数过多".into()));
                    }
                    current_path = build_redirect_path(&current_path, &server_reference);
                }
                ConnackOutcome::Rejected(code) => {
                    return Err(QmError::Network(format!("MQTT Connect Failed. Reason Code: {code:#x}")));
                }
            }
        }
    }

    /// 发送一条原始字节消息 (作为二进制 WebSocket 帧).
    async fn send_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.ws
            .send(Message::Binary(bytes.to_vec()))
            .await
            .map_err(|e| QmError::Network(format!("MQTT 发送失败: {e}")))
    }

    /// 等待并解析 CONNACK.
    async fn wait_connack(&mut self) -> Result<ConnackOutcome> {
        loop {
            let (kind, body) = self.read_packet().await?;
            match kind {
                0x20 => {
                    // CONNACK: session present(1) + reason code(1) + properties
                    let mut pos = 0;
                    let _session_present = body.get(pos).copied().unwrap_or(0);
                    pos += 1;
                    let reason = body.get(pos).copied().unwrap_or(0xFF);
                    pos += 1;
                    let props = parse_properties(&body, &mut pos).unwrap_or_default();
                    if reason == 0x00 {
                        return Ok(ConnackOutcome::Accepted {
                            keep_alive: props.server_keep_alive,
                        });
                    }
                    if (reason == 0x9C || reason == 0x9D) && props.server_reference.is_some() {
                        let reference = props.server_reference.unwrap();
                        self.ws.close(None).await.ok();
                        return Ok(ConnackOutcome::Redirect(reference));
                    }
                    return Ok(ConnackOutcome::Rejected(reason));
                }
                0xD0 => continue, // PINGRESP
                _ => continue,
            }
        }
    }

    /// 订阅主题并等待 SUBACK.
    pub async fn subscribe(&mut self, topic: &str, properties: &MqttProperties) -> Result<()> {
        let packet_id = self.packet_id.fetch_add(1, Ordering::SeqCst).max(1);
        let packet = build_subscribe(packet_id, topic, properties);
        self.send_raw(&packet).await?;

        loop {
            let (kind, body) = self.read_packet().await?;
            match kind {
                0x90 => {
                    // SUBACK: packet id(2) + properties + reason codes
                    let mut pos = 0;
                    let id = u16::from_be_bytes([body.get(pos).copied().unwrap_or(0), body.get(pos + 1).copied().unwrap_or(0)]);
                    pos += 2;
                    let _props_len = read_varint(&body, &mut pos);
                    let mut reasons = Vec::new();
                    while pos < body.len() {
                        reasons.push(body[pos]);
                        pos += 1;
                    }
                    if id != packet_id {
                        continue;
                    }
                    if reasons.iter().any(|&r| r >= 0x80) {
                        return Err(QmError::Network(format!("SUBACK rejected: {reasons:?}")));
                    }
                    return Ok(());
                }
                0xD0 => continue,
                _ => continue,
            }
        }
    }

    /// 读取下一个服务端推送的 PUBLISH 消息.
    pub async fn next_message(&mut self) -> Result<MqttMessage> {
        loop {
            let (kind, body) = self.read_packet().await?;
            match kind {
                0x30 => {
                    // PUBLISH: topic + [packet id] + properties + payload
                    let mut pos = 0;
                    let topic = read_string(&body, &mut pos).ok_or_else(|| QmError::Network("解析 MQTT 主题失败".into()))?;
                    let qos = (kind & 0x06) >> 1;
                    if qos > 0 {
                        pos += 2; // packet id
                    }
                    let props = parse_properties(&body, &mut pos).unwrap_or_default();
                    let payload = body.get(pos..).unwrap_or(&[]).to_vec();
                    let properties: HashMap<String, String> = props.user_property.into_iter().collect();
                    return Ok(MqttMessage {
                        topic,
                        payload,
                        properties,
                    });
                }
                0xE0 => {
                    return Err(QmError::Network("MQTT 连接被服务端关闭 (DISCONNECT)".into()));
                }
                _ => continue,
            }
        }
    }

    /// 从缓冲区解析下一个完整 MQTT 数据包.
    async fn read_packet(&mut self) -> Result<(u8, Vec<u8>)> {
        loop {
            if let Some(packet) = try_parse_packet(&self.recv_buf) {
                let (consumed, kind, body) = packet;
                self.recv_buf.drain(..consumed);
                return Ok((kind, body));
            }
            match self.ws.next().await {
                Some(Ok(Message::Binary(data))) => self.recv_buf.extend(data),
                Some(Ok(Message::Ping(_))) => {
                    self.ws.send(Message::Pong(vec![])).await.ok();
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    return Err(QmError::Network(format!("MQTT 读取失败: {e}")));
                }
                None => {
                    return Err(QmError::Network("MQTT 连接已关闭".into()));
                }
            }
        }
    }

    /// 发送 PINGREQ 心跳.
    #[allow(dead_code)]
    pub async fn ping(&mut self) -> Result<()> {
        self.send_raw(&[0xC0, 0x00]).await
    }

    /// 关闭连接.
    #[allow(dead_code)]
    pub async fn close(&mut self) {
        self.ws.close(None).await.ok();
    }
}

enum ConnackOutcome {
    Accepted {
        keep_alive: Option<u16>,
    },
    Redirect(String),
    Rejected(u8),
}

/// 从字节缓冲区解析一个完整数据包.
fn try_parse_packet(buf: &[u8]) -> Option<(usize, u8, Vec<u8>)> {
    if buf.is_empty() {
        return None;
    }
    let first = buf[0];
    let mut pos = 1;
    let remaining = read_varint(buf, &mut pos)?;
    let total = pos + remaining as usize;
    if buf.len() < total {
        return None;
    }
    Some((total, first & 0xF0, buf[pos..total].to_vec()))
}

/// 根据 serverReference 生成重定向后的握手路径.
fn build_redirect_path(path: &str, server_reference: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if let Some(last) = trimmed.rsplit('/').next() {
        if last.contains(':') {
            let parts: Vec<&str> = trimmed.split('/').collect();
            let mut parts = parts;
            parts.pop();
            parts.push(server_reference);
            return parts.join("/");
        }
    }
    format!("{trimmed}/{server_reference}")
}
