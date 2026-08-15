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
    let mut count = 0;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        count += 1;
        value += ((byte & 0x7F) as u64) * multiplier;
        if byte & 0x80 == 0 {
            break;
        }
        // MQTT 5 Variable Byte Integer 最多 4 字节 (max 268,435,455).
        if count >= 4 {
            return None;
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

/// 读取定长字节串 (Binary Data, 2 字节长度前缀).
fn read_binary(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let len = u16::from_be_bytes([*data.get(*pos)?, *data.get(*pos + 1)?]) as usize;
    *pos += 2;
    let s = data.get(*pos..*pos + len)?;
    *pos += len;
    Some(s.to_vec())
}

/// 跳过未知 MQTT 5.0 属性的值 (按 OASIS MQTT 5 属性注册表确定长度).
///
/// 返回 `false` 表示缓冲不足 / 非法; 成功时 `pos` 前进到属性值末尾.
/// 完整注册表见 `docs.oasis-open.org/mqtt/mqtt/v5.0/os/` 表 2-2.
fn skip_unknown_property(pid: u8, data: &[u8], pos: &mut usize) -> Option<()> {
    let skip: usize = match pid {
        // Byte (1)
        0x01 | 0x17 | 0x19 | 0x24 | 0x25 | 0x28 | 0x29 | 0x2A => 1,
        // Two Byte Integer (2)
        0x13 | 0x21 | 0x22 | 0x23 => 2,
        // Four Byte Integer (4)
        0x02 | 0x11 | 0x18 | 0x27 => 4,
        // UTF-8 Encoded String
        0x03 | 0x08 | 0x12 | 0x15 | 0x1A | 0x1C | 0x1F => {
            let _ = read_string(data, pos)?;
            return Some(());
        }
        // Binary Data
        0x09 | 0x16 => {
            let _ = read_binary(data, pos)?;
            return Some(());
        }
        // Variable Byte Integer
        0x0B => {
            let _ = read_varint(data, pos)?;
            return Some(());
        }
        // UTF-8 String Pair
        0x26 => {
            let _ = read_string(data, pos)?;
            let _ = read_string(data, pos)?;
            return Some(());
        }
        _ => return None,
    };
    if data.len() < *pos + skip {
        return None;
    }
    *pos += skip;
    Some(())
}

fn parse_properties(data: &[u8], pos: &mut usize) -> Option<MqttProperties> {
    let plen = read_varint(data, pos)? as usize;
    let end = (*pos).saturating_add(plen);
    if data.len() < end {
        return None;
    }
    let mut props = MqttProperties::default();
    while *pos < end {
        let pid = read_varint(data, pos)? as u8;
        match pid {
            property_id::SERVER_KEEP_ALIVE => {
                props.server_keep_alive =
                    Some(u16::from_be_bytes([*data.get(*pos)?, *data.get(*pos + 1)?]));
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
                // 未知属性: 按类型跳过值, 而不是把后续内容误当作 payload.
                skip_unknown_property(pid, data, pos)?;
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
            let mut request = url
                .into_client_request()
                .map_err(|e| QmError::Network(e.to_string()))?;
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
                    return Err(QmError::Network(format!(
                        "MQTT Connect Failed. Reason Code: {code:#x}"
                    )));
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
            match kind & 0xF0 {
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
                    if reason == 0x9C || reason == 0x9D {
                        if let Some(reference) = props.server_reference {
                            self.ws.close(None).await.ok();
                            return Ok(ConnackOutcome::Redirect(reference));
                        }
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
            match kind & 0xF0 {
                0x90 => {
                    // SUBACK: packet id(2) + properties + reason codes
                    let mut pos = 0;
                    let id = u16::from_be_bytes([
                        body.get(pos).copied().unwrap_or(0),
                        body.get(pos + 1).copied().unwrap_or(0),
                    ]);
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
            match kind & 0xF0 {
                0x30 => {
                    // PUBLISH: topic + [packet id] + properties + payload
                    let mut pos = 0;
                    let topic = read_string(&body, &mut pos)
                        .ok_or_else(|| QmError::Network("解析 MQTT 主题失败".into()))?;
                    let qos = (kind & 0x06) >> 1;
                    if qos > 0 {
                        pos += 2; // packet id
                    }
                    let props = parse_properties(&body, &mut pos).unwrap_or_default();
                    let payload = body.get(pos..).unwrap_or(&[]).to_vec();
                    let properties: HashMap<String, String> =
                        props.user_property.into_iter().collect();
                    return Ok(MqttMessage {
                        topic,
                        payload,
                        properties,
                    });
                }
                0xE0 => {
                    return Err(QmError::Network(
                        "MQTT 连接被服务端关闭 (DISCONNECT)".into(),
                    ));
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
    Accepted { keep_alive: Option<u16> },
    Redirect(String),
    Rejected(u8),
}

/// 从字节缓冲区解析一个完整数据包.
///
/// 返回 `(consumed, first_byte, body)`. `first_byte` 保留完整 Fixed Header,
/// 其中低 4 位是标志位 (PUBLISH 的 QoS 等), 调用方按需 `& 0xF0` 取类型.
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
    Some((total, first, buf[pos..total].to_vec()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_props_len(len: usize) -> Vec<u8> {
        encode_varint(len as u64)
    }

    /// 构造一个 PUBLISH 数据包.
    fn build_publish(
        qos: u8,
        topic: &str,
        packet_id: Option<u16>,
        props: &[u8],
        payload: &[u8],
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(encode_string(topic));
        if qos > 0 {
            body.extend(packet_id.unwrap_or(1).to_be_bytes());
        }
        body.extend(encode_props_len(props.len()));
        body.extend(props);
        body.extend(payload);
        let first = 0x30 | (qos << 1);
        let mut packet = vec![first];
        packet.extend(encode_varint(body.len() as u64));
        packet.extend(body);
        packet
    }

    #[test]
    fn try_parse_packet_preserves_qos_flags() {
        // QoS 1 PUBLISH: first byte 0x32.
        let packet = build_publish(1, "topic/a", Some(7), &[], b"hello");
        let (consumed, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(consumed, packet.len());
        assert_eq!(first, 0x32);
        // 低 4 位标志位保留, QoS 可从完整字节中读取.
        assert_eq!((first & 0x06) >> 1, 1);
        assert_eq!(body.len(), packet.len() - 2); // fixed header = first byte + varint
    }

    #[test]
    fn qos1_publish_payload_offset() {
        // 模拟 next_message 的解析逻辑, 验证 QoS 1 时跳过 2 字节 packet id 后 payload 正确.
        let topic = "management.qrcode_login/abc";
        let payload = br#"{"type":"scanned"}"#;
        let packet = build_publish(1, topic, Some(7), &[], payload);
        let (_, first, body) = try_parse_packet(&packet).unwrap();

        let mut pos = 0;
        let parsed_topic = read_string(&body, &mut pos).unwrap();
        assert_eq!(parsed_topic, topic);
        let qos = (first & 0x06) >> 1;
        assert_eq!(qos, 1);
        if qos > 0 {
            pos += 2;
        }
        let props = parse_properties(&body, &mut pos).unwrap_or_default();
        assert!(props.user_property.is_empty());
        assert_eq!(body.get(pos..).unwrap(), payload);
    }

    #[test]
    fn qos0_publish_no_packet_id() {
        let payload = br#"{"a":1}"#;
        let packet = build_publish(0, "t", None, &[], payload);
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        let mut pos = 0;
        let _ = read_string(&body, &mut pos).unwrap();
        let qos = (first & 0x06) >> 1;
        assert_eq!(qos, 0);
        let _props = parse_properties(&body, &mut pos).unwrap_or_default();
        assert_eq!(body.get(pos..).unwrap(), payload);
    }

    #[test]
    fn parse_properties_skips_unknown_and_reads_user_property() {
        // 未知属性 0x22 (Topic Alias Maximum, 2 字节) + 用户属性 type=scanned.
        let mut props_bytes = Vec::new();
        props_bytes.push(0x22);
        props_bytes.extend(7u16.to_be_bytes());
        props_bytes.push(property_id::USER_PROPERTY);
        props_bytes.extend(encode_string("type"));
        props_bytes.extend(encode_string("scanned"));
        let packet = build_publish(0, "t", None, &props_bytes, b"PAYLOAD");
        let (_, _, body) = try_parse_packet(&packet).unwrap();
        let mut pos = 0;
        let _ = read_string(&body, &mut pos).unwrap();
        let props = parse_properties(&body, &mut pos).unwrap();
        assert_eq!(
            props.user_property,
            vec![("type".to_string(), "scanned".to_string())]
        );
        // 未知属性已被跳过, payload 起点正确.
        assert_eq!(body.get(pos..).unwrap(), b"PAYLOAD");
    }

    #[test]
    fn unknown_property_type_table_matches_oasis() {
        // 逐一验证 OASIS MQTT 5 注册表中"未显式处理"属性的长度跳转.
        let cases: &[(u8, usize)] = &[
            // Byte (1)
            (0x01, 1),
            (0x17, 1),
            (0x19, 1),
            (0x24, 1),
            (0x25, 1),
            (0x28, 1),
            (0x29, 1),
            (0x2A, 1),
            // Two Byte (2)
            (0x13, 2),
            (0x21, 2),
            (0x22, 2),
            (0x23, 2),
            // Four Byte (4)
            (0x02, 4),
            (0x11, 4),
            (0x18, 4),
            (0x27, 4),
        ];
        for (pid, len) in cases {
            let value = vec![0xAB; *len];
            let mut buf = vec![*pid];
            buf.extend(&value);
            // parse_properties 已消费属性 ID 字节, pos 从 1 开始指向值.
            let mut pos = 1;
            assert!(
                skip_unknown_property(*pid, &buf, &mut pos).is_some(),
                "pid {pid:#x} 应可跳过"
            );
            assert_eq!(pos, 1 + len, "pid {pid:#x} 长度应为 {len}");
        }
    }

    #[test]
    fn unknown_property_requires_enough_bytes() {
        // 缓冲不足时不得推进/panic.
        let mut pos = 0;
        assert!(skip_unknown_property(0x27, &[0x27, 0x00], &mut pos).is_none());
        assert_eq!(pos, 0);
    }

    #[test]
    fn varint_enforces_four_byte_limit() {
        // 268,435,455 = 0xFF 0xFF 0xFF 0x7F 是合法最大值 (4 字节).
        let max = encode_varint(268_435_455);
        assert_eq!(max.len(), 4);
        let mut pos = 0;
        assert_eq!(read_varint(&max, &mut pos), Some(268_435_455));

        // 超出 4 字节 (如 268_435_456) 属 malformed, 应返回 None 而非无限读取.
        let over = encode_varint(268_435_456);
        assert_eq!(over.len(), 5);
        let mut pos = 0;
        assert_eq!(read_varint(&over, &mut pos), None);
    }

    #[test]
    fn varint_roundtrip() {
        for v in [
            0u64,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            2_097_151,
            268_435_455,
        ] {
            let enc = encode_varint(v);
            let mut pos = 0;
            let dec = read_varint(&enc, &mut pos).unwrap();
            assert_eq!(dec, v);
            assert_eq!(pos, enc.len());
        }
    }

    #[test]
    fn build_connect_roundtrips_properties() {
        let props = MqttProperties::default()
            .auth_method("pass")
            .user_property(&[("tmeAppID", "qqmusic")]);
        let packet = build_connect("client-1", 45, &props);
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(first & 0xF0, 0x10); // CONNECT
                                        // protocol name
        let mut pos = 0;
        let proto = read_string(&body, &mut pos).unwrap();
        assert_eq!(proto, "MQTT");
        assert_eq!(body[pos], 5); // version
        pos += 1;
        pos += 1; // connect flags
        pos += 2; // keep alive
        let parsed = parse_properties(&body, &mut pos).unwrap();
        assert_eq!(parsed.auth_method.as_deref(), Some("pass"));
        assert_eq!(
            parsed.user_property,
            vec![("tmeAppID".to_string(), "qqmusic".to_string())]
        );
        let cid = read_string(&body, &mut pos).unwrap();
        assert_eq!(cid, "client-1");
    }

    #[test]
    fn subscribe_build_and_parse_reasons() {
        let props = MqttProperties::default().user_property(&[("authorization", "tmelogin")]);
        let packet = build_subscribe(3, "management.qrcode_login/x", &props);
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(first & 0xF0, 0x80); // SUBSCRIBE (flags 0x2 保留在低 4 位)
        assert_eq!(first & 0x0F, 0x02);
        let mut pos = 0;
        let id = u16::from_be_bytes([body[pos], body[pos + 1]]);
        pos += 2;
        assert_eq!(id, 3);
        let _ = parse_properties(&body, &mut pos).unwrap();
        let topic = read_string(&body, &mut pos).unwrap();
        assert_eq!(topic, "management.qrcode_login/x");
        assert_eq!(body[pos], 0); // QoS 0
    }
}
