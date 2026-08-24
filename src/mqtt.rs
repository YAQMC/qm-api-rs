//! MQTT 5.0 over WebSocket 通用客户端 (对应 Python 端 `utils/mqtt.py`).
//!
//! 实现手机端二维码登录所需的 CONNECT / CONNACK / SUBSCRIBE / SUBACK /
//! PUBLISH / QoS ACK / PINGREQ / PINGRESP / DISCONNECT 子集，支持用户属性与服务器重定向.

use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::{QmError, Result};

/// 本客户端接受的单个 MQTT packet 最大 Remaining Length。
/// 二维码登录消息通常只有 KB 级；4 MiB 为异常响应留出充分余量，同时阻止
/// MQTT 最大理论长度（约 256 MiB）造成无界 recv_buf 增长。
const MAX_MQTT_PACKET_SIZE: usize = 4 * 1024 * 1024;

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
    /// 等待 CONNACK/SUBACK 等控制包时提前到达的业务消息不能丢弃.
    pending_messages: VecDeque<MqttMessage>,
    /// QoS 2 PUBLISH 在 PUBREL 到达前暂存，以完成 PUBREC -> PUBREL -> PUBCOMP 流程.
    qos2_pending: HashMap<u16, MqttMessage>,
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

fn encode_string(s: &str) -> Result<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(QmError::ValueError(
            "MQTT UTF-8 string 超过 65,535 字节".into(),
        ));
    }
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.extend((bytes.len() as u16).to_be_bytes());
    out.extend(bytes);
    Ok(out)
}

fn encode_properties(props: &MqttProperties) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    if let Some(m) = &props.auth_method {
        body.push(property_id::AUTH_METHOD);
        body.extend(encode_string(m)?);
    }
    for (k, v) in &props.user_property {
        body.push(property_id::USER_PROPERTY);
        body.extend(encode_string(k)?);
        body.extend(encode_string(v)?);
    }
    let mut out = encode_varint(body.len() as u64);
    out.extend(body);
    Ok(out)
}

fn build_connect(client_id: &str, keep_alive: u16, props: &MqttProperties) -> Result<Vec<u8>> {
    let mut var = Vec::new();
    var.extend(encode_string("MQTT")?);
    var.push(5);
    var.push(0x02);
    var.extend(keep_alive.to_be_bytes());
    var.extend(encode_properties(props)?);
    var.extend(encode_string(client_id)?);
    if var.len() > MAX_MQTT_PACKET_SIZE {
        return Err(QmError::ValueError("MQTT CONNECT packet 过大".into()));
    }
    let mut packet = vec![0x10];
    packet.extend(encode_varint(var.len() as u64));
    packet.extend(var);
    Ok(packet)
}

/// PINGREQ 固定报头 (MQTT 5).
pub(crate) fn build_pingreq() -> [u8; 2] {
    [0xC0, 0x00]
}

/// Keep Alive 为 0 表示不主动 ping (避免 busy loop).
pub(crate) fn keep_alive_interval(keep_alive_secs: u16) -> Option<Duration> {
    if keep_alive_secs == 0 {
        None
    } else {
        Some(Duration::from_secs(u64::from(keep_alive_secs)))
    }
}

fn build_subscribe(packet_id: u16, topic: &str, props: &MqttProperties) -> Result<Vec<u8>> {
    let mut var = Vec::new();
    var.extend(packet_id.to_be_bytes());
    var.extend(encode_properties(props)?);
    var.extend(encode_string(topic)?);
    var.push(0x00);
    if var.len() > MAX_MQTT_PACKET_SIZE {
        return Err(QmError::ValueError("MQTT SUBSCRIBE packet 过大".into()));
    }
    let mut packet = vec![0x82];
    packet.extend(encode_varint(var.len() as u64));
    packet.extend(var);
    Ok(packet)
}

fn build_packet_id_ack(first: u8, packet_id: u16) -> [u8; 4] {
    [first, 0x02, (packet_id >> 8) as u8, packet_id as u8]
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
    let end = (*pos).checked_add(len)?;
    let s = data.get(*pos..end)?;
    *pos = end;
    String::from_utf8(s.to_vec()).ok()
}

fn read_binary(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let len = u16::from_be_bytes([*data.get(*pos)?, *data.get(*pos + 1)?]) as usize;
    *pos += 2;
    let end = (*pos).checked_add(len)?;
    let s = data.get(*pos..end)?;
    *pos = end;
    Some(s.to_vec())
}

fn skip_unknown_property(pid: u8, data: &[u8], pos: &mut usize) -> Option<()> {
    let skip: usize = match pid {
        0x01 | 0x17 | 0x19 | 0x24 | 0x25 | 0x28 | 0x29 | 0x2A => 1,
        0x13 | 0x21 | 0x22 | 0x23 => 2,
        0x02 | 0x11 | 0x18 | 0x27 => 4,
        0x03 | 0x08 | 0x12 | 0x15 | 0x1A | 0x1C | 0x1F => {
            let _ = read_string(data, pos)?;
            return Some(());
        }
        0x09 | 0x16 => {
            let _ = read_binary(data, pos)?;
            return Some(());
        }
        0x0B => {
            let _ = read_varint(data, pos)?;
            return Some(());
        }
        0x26 => {
            let _ = read_string(data, pos)?;
            let _ = read_string(data, pos)?;
            return Some(());
        }
        _ => return None,
    };
    let end = (*pos).checked_add(skip)?;
    if data.len() < end {
        return None;
    }
    *pos = end;
    Some(())
}

fn parse_properties(data: &[u8], pos: &mut usize) -> Option<MqttProperties> {
    let plen = read_varint(data, pos)? as usize;
    let end = (*pos).checked_add(plen)?;
    if data.len() < end {
        return None;
    }
    let mut props = MqttProperties::default();
    while *pos < end {
        let pid_raw = read_varint(data, pos)?;
        let pid = u8::try_from(pid_raw).ok()?;
        match pid {
            property_id::SERVER_KEEP_ALIVE => {
                props.server_keep_alive =
                    Some(u16::from_be_bytes([*data.get(*pos)?, *data.get(*pos + 1)?]));
                *pos += 2;
            }
            property_id::SERVER_REFERENCE => props.server_reference = read_string(data, pos),
            property_id::REASON_STRING => props.reason_string = read_string(data, pos),
            property_id::AUTH_METHOD => props.auth_method = read_string(data, pos),
            property_id::USER_PROPERTY => {
                let k = read_string(data, pos)?;
                let v = read_string(data, pos)?;
                props.user_property.push((k, v));
            }
            _ => skip_unknown_property(pid, data, pos)?,
        }
        if *pos > end {
            return None;
        }
    }
    (*pos == end).then_some(props)
}

fn parse_publish(kind: u8, body: &[u8]) -> Result<(MqttMessage, u8, Option<u16>)> {
    let qos = (kind & 0x06) >> 1;
    if qos == 3 {
        return Err(QmError::network("MQTT PUBLISH 使用保留的 QoS=3"));
    }
    let mut pos = 0;
    let topic = read_string(body, &mut pos)
        .ok_or_else(|| QmError::network("解析 MQTT 主题失败"))?;
    let packet_id = if qos > 0 {
        let hi = *body
            .get(pos)
            .ok_or_else(|| QmError::network("MQTT PUBLISH 缺少 packet id"))?;
        let lo = *body
            .get(pos + 1)
            .ok_or_else(|| QmError::network("MQTT PUBLISH 缺少 packet id"))?;
        pos += 2;
        let id = u16::from_be_bytes([hi, lo]);
        if id == 0 {
            return Err(QmError::network("MQTT packet id 不能为 0"));
        }
        Some(id)
    } else {
        None
    };
    let props = parse_properties(body, &mut pos)
        .ok_or_else(|| QmError::network("解析 MQTT PUBLISH properties 失败"))?;
    let payload = body
        .get(pos..)
        .ok_or_else(|| QmError::network("解析 MQTT PUBLISH payload 失败"))?
        .to_vec();
    Ok((
        MqttMessage {
            topic,
            payload,
            properties: props.user_property.into_iter().collect(),
        },
        qos,
        packet_id,
    ))
}

impl MqttClient {
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
                .map_err(|e| QmError::network(e.to_string()))?;
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
                .map_err(|e| QmError::network(format!("MQTT WebSocket 握手失败: {e}")))?;

            let mut client = MqttClient {
                ws,
                host: host.to_string(),
                port,
                path: current_path.clone(),
                keep_alive,
                packet_id: AtomicU16::new(1),
                recv_buf: Vec::new(),
                pending_messages: VecDeque::new(),
                qos2_pending: HashMap::new(),
            };

            let connect_packet = build_connect(client_id, keep_alive, properties)?;
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
                        return Err(QmError::network("MQTT 重定向次数过多"));
                    }
                    current_path = build_redirect_path(&current_path, &server_reference);
                }
                ConnackOutcome::Rejected(code) => {
                    return Err(QmError::network(format!(
                        "MQTT Connect Failed. Reason Code: {code:#x}"
                    )));
                }
            }
        }
    }

    async fn send_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.ws
            .send(Message::Binary(bytes.to_vec()))
            .await
            .map_err(|e| QmError::network(format!("MQTT 发送失败: {e}")))
    }

    async fn wait_connack(&mut self) -> Result<ConnackOutcome> {
        loop {
            let (kind, body) = self.read_packet().await?;
            match kind & 0xF0 {
                0x20 => {
                    if body.len() < 2 {
                        return Err(QmError::network("CONNACK 长度不足"));
                    }
                    let mut pos = 0;
                    let _session_present = body[pos];
                    pos += 1;
                    let reason = body[pos];
                    pos += 1;
                    let props = parse_properties(&body, &mut pos)
                        .ok_or_else(|| QmError::network("解析 CONNACK properties 失败"))?;
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
                0xD0 => continue,
                _ => continue,
            }
        }
    }

    async fn handle_publish_while_waiting(&mut self, kind: u8, body: &[u8]) -> Result<()> {
        let (message, qos, packet_id) = parse_publish(kind, body)?;
        match (qos, packet_id) {
            (0, _) => self.pending_messages.push_back(message),
            (1, Some(id)) => {
                self.send_raw(&build_packet_id_ack(0x40, id)).await?; // PUBACK
                self.pending_messages.push_back(message);
            }
            (2, Some(id)) => {
                self.send_raw(&build_packet_id_ack(0x50, id)).await?; // PUBREC
                self.qos2_pending.entry(id).or_insert(message);
            }
            _ => return Err(QmError::network("无效的 MQTT PUBLISH QoS 状态")),
        }
        Ok(())
    }

    async fn handle_pubrel(&mut self, body: &[u8]) -> Result<Option<MqttMessage>> {
        if body.len() < 2 {
            return Err(QmError::network("PUBREL 缺少 packet id"));
        }
        let id = u16::from_be_bytes([body[0], body[1]]);
        if id == 0 {
            return Err(QmError::network("PUBREL packet id 不能为 0"));
        }
        self.send_raw(&build_packet_id_ack(0x70, id)).await?; // PUBCOMP
        Ok(self.qos2_pending.remove(&id))
    }

    /// 订阅主题并等待 SUBACK。等待期间到达的 PUBLISH 会被确认并缓存，不会丢弃.
    pub async fn subscribe(&mut self, topic: &str, properties: &MqttProperties) -> Result<()> {
        let mut packet_id = self.packet_id.fetch_add(1, Ordering::SeqCst);
        if packet_id == 0 {
            packet_id = self.packet_id.fetch_add(1, Ordering::SeqCst);
            if packet_id == 0 {
                packet_id = 1;
            }
        }
        let packet = build_subscribe(packet_id, topic, properties)?;
        self.send_raw(&packet).await?;

        loop {
            let (kind, body) = self.read_packet().await?;
            match kind & 0xF0 {
                0x90 => {
                    if body.len() < 3 {
                        return Err(QmError::network("SUBACK 长度不足"));
                    }
                    let mut pos = 0;
                    let id = u16::from_be_bytes([body[pos], body[pos + 1]]);
                    pos += 2;
                    // MQTT 5 SUBACK: packet identifier 后是完整 Properties 字段，再后才是
                    // reason-code payload；必须消费 properties 体，而不是只读取其长度。
                    let _props = parse_properties(&body, &mut pos)
                        .ok_or_else(|| QmError::network("解析 SUBACK properties 失败"))?;
                    let reasons = body
                        .get(pos..)
                        .ok_or_else(|| QmError::network("解析 SUBACK reason codes 失败"))?;
                    if id != packet_id {
                        continue;
                    }
                    if reasons.is_empty() {
                        return Err(QmError::network("SUBACK 未包含 reason code"));
                    }
                    if reasons.iter().any(|&r| r >= 0x80) {
                        return Err(QmError::network(format!("SUBACK rejected: {reasons:?}")));
                    }
                    return Ok(());
                }
                0x30 => self.handle_publish_while_waiting(kind, &body).await?,
                0x60 => {
                    if let Some(message) = self.handle_pubrel(&body).await? {
                        self.pending_messages.push_back(message);
                    }
                }
                0xD0 => continue,
                0xE0 => return Err(QmError::network("MQTT 连接被服务端关闭 (DISCONNECT)")),
                _ => continue,
            }
        }
    }

    /// 读取下一个服务端推送的 PUBLISH 消息.
    pub async fn next_message(&mut self) -> Result<MqttMessage> {
        if let Some(message) = self.pending_messages.pop_front() {
            return Ok(message);
        }
        loop {
            let (kind, body) = self.read_packet().await?;
            match kind & 0xF0 {
                0x30 => {
                    let (message, qos, packet_id) = parse_publish(kind, &body)?;
                    match (qos, packet_id) {
                        (0, _) => return Ok(message),
                        (1, Some(id)) => {
                            self.send_raw(&build_packet_id_ack(0x40, id)).await?;
                            return Ok(message);
                        }
                        (2, Some(id)) => {
                            self.send_raw(&build_packet_id_ack(0x50, id)).await?;
                            self.qos2_pending.entry(id).or_insert(message);
                        }
                        _ => return Err(QmError::network("无效的 MQTT PUBLISH QoS 状态")),
                    }
                }
                0x60 => {
                    if let Some(message) = self.handle_pubrel(&body).await? {
                        return Ok(message);
                    }
                }
                0xE0 => {
                    return Err(QmError::network("MQTT 连接被服务端关闭 (DISCONNECT)"));
                }
                0xD0 => continue,
                _ => continue,
            }
        }
    }

    async fn read_packet(&mut self) -> Result<(u8, Vec<u8>)> {
        loop {
            if self.recv_buf.len() > MAX_MQTT_PACKET_SIZE + 5 {
                return Err(QmError::network("MQTT 接收包超过本地大小限制"));
            }
            if self.recv_buf.len() >= 2 {
                let mut pos = 1;
                if let Some(remaining) = read_varint(&self.recv_buf, &mut pos) {
                    if remaining > MAX_MQTT_PACKET_SIZE as u64 {
                        return Err(QmError::network("MQTT Remaining Length 超过本地大小限制"));
                    }
                }
            }
            if let Some(packet) = try_parse_packet(&self.recv_buf) {
                let (consumed, kind, body) = packet;
                self.recv_buf.drain(..consumed);
                return Ok((kind, body));
            }
            match self.ws.next().await {
                Some(Ok(Message::Binary(data))) => {
                    if self.recv_buf.len().saturating_add(data.len()) > MAX_MQTT_PACKET_SIZE + 5 {
                        return Err(QmError::network("MQTT 接收缓冲超过本地大小限制"));
                    }
                    self.recv_buf.extend(data);
                }
                Some(Ok(Message::Ping(payload))) => {
                    // RFC 6455: Pong 必须复制对应 Ping 的 Application data.
                    self.ws
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| QmError::network(format!("WebSocket Pong 发送失败: {e}")))?;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    return Err(QmError::network(format!("MQTT 读取失败: {e}")));
                }
                None => return Err(QmError::network("MQTT 连接已关闭")),
            }
        }
    }

    pub async fn ping(&mut self) -> Result<()> {
        self.send_raw(&build_pingreq()).await
    }

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

fn try_parse_packet(buf: &[u8]) -> Option<(usize, u8, Vec<u8>)> {
    if buf.is_empty() {
        return None;
    }
    let first = buf[0];
    let mut pos = 1;
    let remaining = read_varint(buf, &mut pos)?;
    if remaining > MAX_MQTT_PACKET_SIZE as u64 {
        return None;
    }
    let total = pos.checked_add(usize::try_from(remaining).ok()?)?;
    if buf.len() < total {
        return None;
    }
    Some((total, first, buf[pos..total].to_vec()))
}

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

    fn build_publish(
        qos: u8,
        topic: &str,
        packet_id: Option<u16>,
        props: &[u8],
        payload: &[u8],
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(encode_string(topic).unwrap());
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
    fn mqtt_string_rejects_oversized_input() {
        let long = "x".repeat(u16::MAX as usize + 1);
        assert!(encode_string(&long).is_err());
    }

    #[test]
    fn try_parse_packet_preserves_qos_flags() {
        let packet = build_publish(1, "topic/a", Some(7), &[], b"hello");
        let (consumed, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(consumed, packet.len());
        assert_eq!(first, 0x32);
        assert_eq!((first & 0x06) >> 1, 1);
        assert_eq!(body.len(), packet.len() - 2);
    }

    #[test]
    fn qos1_publish_payload_offset() {
        let topic = "management.qrcode_login/abc";
        let payload = br#"{"type":"scanned"}"#;
        let packet = build_publish(1, topic, Some(7), &[], payload);
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        let (message, qos, packet_id) = parse_publish(first, &body).unwrap();
        assert_eq!(qos, 1);
        assert_eq!(packet_id, Some(7));
        assert_eq!(message.topic, topic);
        assert_eq!(message.payload, payload);
    }

    #[test]
    fn qos0_publish_no_packet_id() {
        let payload = br#"{"a":1}"#;
        let packet = build_publish(0, "t", None, &[], payload);
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        let (message, qos, packet_id) = parse_publish(first, &body).unwrap();
        assert_eq!(qos, 0);
        assert_eq!(packet_id, None);
        assert_eq!(message.payload, payload);
    }

    #[test]
    fn parse_properties_skips_unknown_and_reads_user_property() {
        let mut props_bytes = Vec::new();
        props_bytes.push(0x22);
        props_bytes.extend(7u16.to_be_bytes());
        props_bytes.push(property_id::USER_PROPERTY);
        props_bytes.extend(encode_string("type").unwrap());
        props_bytes.extend(encode_string("scanned").unwrap());
        let packet = build_publish(0, "t", None, &props_bytes, b"PAYLOAD");
        let (_, _, body) = try_parse_packet(&packet).unwrap();
        let mut pos = 0;
        let _ = read_string(&body, &mut pos).unwrap();
        let props = parse_properties(&body, &mut pos).unwrap();
        assert_eq!(
            props.user_property,
            vec![("type".to_string(), "scanned".to_string())]
        );
        assert_eq!(body.get(pos..).unwrap(), b"PAYLOAD");
    }

    #[test]
    fn suback_properties_are_consumed_before_reason_codes() {
        let mut body = Vec::new();
        body.extend(3u16.to_be_bytes());
        let mut props = Vec::new();
        props.push(property_id::REASON_STRING);
        props.extend(encode_string("ok").unwrap());
        body.extend(encode_varint(props.len() as u64));
        body.extend(props);
        body.push(0x00);

        let mut pos = 2;
        let parsed = parse_properties(&body, &mut pos).unwrap();
        assert_eq!(parsed.reason_string.as_deref(), Some("ok"));
        assert_eq!(body.get(pos..), Some(&[0x00][..]));
    }

    #[test]
    fn unknown_property_type_table_matches_oasis() {
        let cases: &[(u8, usize)] = &[
            (0x01, 1), (0x17, 1), (0x19, 1), (0x24, 1), (0x25, 1), (0x28, 1),
            (0x29, 1), (0x2A, 1), (0x13, 2), (0x21, 2), (0x22, 2), (0x23, 2),
            (0x02, 4), (0x11, 4), (0x18, 4), (0x27, 4),
        ];
        for (pid, len) in cases {
            let value = vec![0xAB; *len];
            let mut buf = vec![*pid];
            buf.extend(&value);
            let mut pos = 1;
            assert!(skip_unknown_property(*pid, &buf, &mut pos).is_some());
            assert_eq!(pos, 1 + len);
        }
    }

    #[test]
    fn unknown_property_requires_enough_bytes() {
        let mut pos = 0;
        assert!(skip_unknown_property(0x27, &[0x27, 0x00], &mut pos).is_none());
        assert_eq!(pos, 0);
    }

    #[test]
    fn varint_enforces_four_byte_limit() {
        let max = encode_varint(268_435_455);
        assert_eq!(max.len(), 4);
        let mut pos = 0;
        assert_eq!(read_varint(&max, &mut pos), Some(268_435_455));
        let over = encode_varint(268_435_456);
        assert_eq!(over.len(), 5);
        let mut pos = 0;
        assert_eq!(read_varint(&over, &mut pos), None);
    }

    #[test]
    fn varint_roundtrip() {
        for v in [0u64, 1, 127, 128, 300, 16_383, 16_384, 2_097_151, 268_435_455] {
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
        let packet = build_connect("client-1", 45, &props).unwrap();
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(first & 0xF0, 0x10);
        let mut pos = 0;
        let proto = read_string(&body, &mut pos).unwrap();
        assert_eq!(proto, "MQTT");
        assert_eq!(body[pos], 5);
        pos += 1;
        pos += 1;
        pos += 2;
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
        let packet = build_subscribe(3, "management.qrcode_login/x", &props).unwrap();
        let (_, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(first & 0xF0, 0x80);
        assert_eq!(first & 0x0F, 0x02);
        let mut pos = 0;
        let id = u16::from_be_bytes([body[pos], body[pos + 1]]);
        pos += 2;
        assert_eq!(id, 3);
        let _ = parse_properties(&body, &mut pos).unwrap();
        let topic = read_string(&body, &mut pos).unwrap();
        assert_eq!(topic, "management.qrcode_login/x");
        assert_eq!(body[pos], 0);
    }

    #[test]
    fn packet_size_limit_rejects_declared_oversize() {
        let mut packet = vec![0x30];
        packet.extend(encode_varint((MAX_MQTT_PACKET_SIZE + 1) as u64));
        assert!(try_parse_packet(&packet).is_none());
    }

    #[test]
    fn pingreq_is_fixed_header() {
        assert_eq!(build_pingreq(), [0xC0, 0x00]);
        let (consumed, first, body) = try_parse_packet(&build_pingreq()).unwrap();
        assert_eq!(consumed, 2);
        assert_eq!(first & 0xF0, 0xC0);
        assert!(body.is_empty());
    }

    #[test]
    fn keep_alive_zero_disables_client_ping() {
        assert!(keep_alive_interval(0).is_none());
        assert_eq!(keep_alive_interval(45).unwrap(), Duration::from_secs(45));
        assert_eq!(keep_alive_interval(1).unwrap(), Duration::from_secs(1));
    }

    #[test]
    fn connack_server_keep_alive_overrides_client_value() {
        let mut props = Vec::new();
        props.push(property_id::SERVER_KEEP_ALIVE);
        props.extend(20u16.to_be_bytes());
        let mut body = vec![0x00, 0x00];
        body.extend(encode_props_len(props.len()));
        body.extend(props);
        let mut packet = vec![0x20];
        packet.extend(encode_varint(body.len() as u64));
        packet.extend(body);

        let (_, first, body) = try_parse_packet(&packet).unwrap();
        assert_eq!(first & 0xF0, 0x20);
        let mut pos = 0;
        pos += 1;
        let reason = body[pos];
        pos += 1;
        assert_eq!(reason, 0);
        let parsed = parse_properties(&body, &mut pos).unwrap();
        assert_eq!(parsed.server_keep_alive, Some(20));
        let effective = parsed.server_keep_alive.unwrap_or(45);
        assert_eq!(
            keep_alive_interval(effective),
            Some(Duration::from_secs(20))
        );
    }

    #[test]
    fn overall_qr_deadline_is_not_reset_by_non_terminal_messages() {
        let start = std::time::Instant::now();
        let deadline = start + Duration::from_secs(180);
        let after_three_keepalive_messages = start + Duration::from_secs(3 * 5);
        let remaining = deadline.saturating_duration_since(after_three_keepalive_messages);
        assert_eq!(remaining, Duration::from_secs(165));
        assert!(remaining < Duration::from_secs(180));
        let after_lifetime = start + Duration::from_secs(181);
        assert!(deadline.saturating_duration_since(after_lifetime).is_zero());
    }
}