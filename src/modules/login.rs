//! 登录相关业务接口 (对应 Python 端 `modules/login.py`).

use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::login::*;
use crate::models::Credential;
use crate::reply::CgiReply;
use crate::utils::hash33;
use crate::versioning::Platform;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn extract_quoted(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = args.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            let mut s = String::new();
            while let Some(&n) = chars.peek() {
                if n == '\\' {
                    chars.next();
                    if let Some(escaped) = chars.next() {
                        s.push(escaped);
                    }
                } else if n == '\'' {
                    chars.next();
                    break;
                } else {
                    s.push(n);
                    chars.next();
                }
            }
            out.push(s);
        }
    }
    out
}

/// 登录相关的 API.
#[derive(Clone, Debug)]
pub struct LoginApi {
    pub(crate) base: ApiModule,
}

impl LoginApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        LoginApi {
            base: ApiModule::new(context),
        }
    }

    fn validate_result(reply: crate::reply::CgiReply<Value>) -> Result<Credential> {
        let CgiReply { code, data } = reply;
        match code {
            0 => serde_json::from_value(data).map_err(QmError::from),
            1000 | 104401 | 104400 => Err(QmError::CredentialExpired(format!(
                "登录鉴权参数无效或已过期 (code {code})"
            ))),
            20261 => Err(QmError::Login {
                message: "登录参数错误".into(),
                code,
            }),
            20271 => Err(QmError::Login {
                message: "验证码错误".into(),
                code,
            }),
            20272 => Err(QmError::Login {
                message: "账号绑定异常".into(),
                code,
            }),
            20274 => Err(QmError::Login {
                message: "账号绑定缺失".into(),
                code,
            }),
            20277 | 20278 => Err(QmError::Login {
                message: "账号受限".into(),
                code,
            }),
            20279 => Err(QmError::Login {
                message: "登录设备数量超限".into(),
                code,
            }),
            20450 => Err(QmError::Login {
                message: "账号已被封禁".into(),
                code,
            }),
            104604 => Err(QmError::Login {
                message: "操作过于频繁".into(),
                code,
            }),
            _ => Err(QmError::Login {
                message: format!(
                    "登录失败: {}",
                    crate::error::redact_payload(&data.to_string(), 200)
                ),
                code,
            }),
        }
    }

    /// 检查登录凭证是否已过期.
    pub async fn check_expired(&self, credential: Option<&Credential>) -> Result<bool> {
        let target = credential
            .cloned()
            .unwrap_or_else(|| self.base.credential());
        if self.base.context.platform == Platform::Web {
            let opts = crate::client::HttpOptions {
                params: vec![
                    (
                        "g_tk".to_string(),
                        hash33(&target.musickey, 5381).to_string(),
                    ),
                    ("format".to_string(), "json".to_string()),
                    ("inCharset".to_string(), "utf-8".to_string()),
                    ("outCharset".to_string(), "utf-8".to_string()),
                    ("notice".to_string(), "0".to_string()),
                    ("cid".to_string(), "205360838".to_string()),
                    ("needNewCode".to_string(), "0".to_string()),
                    ("loginUin".to_string(), target.str_musicid()),
                    ("hostUin".to_string(), "0".to_string()),
                    ("userid".to_string(), target.str_musicid()),
                    ("reqfrom".to_string(), "1".to_string()),
                ],
                credential: Some(target),
                ..Default::default()
            };
            let text = self
                .base
                .context
                .request_http(
                    reqwest::Method::GET,
                    "https://c6.y.qq.com/rsc/fcgi-bin/fcg_get_profile_homepage.fcg",
                    &opts,
                )
                .await?;
            let value: Value = serde_json::from_str(&text)?;
            let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
            // 仅已知的凭证过期码视为过期; 未知 code 返回错误而非一律视为过期.
            if code == 0 {
                return Ok(false);
            }
            if [1000, 104401, 104400].contains(&code) {
                return Ok(true);
            }
            return Err(QmError::Login {
                message: format!(
                    "检查过期失败: {}",
                    crate::error::redact_payload(&value.to_string(), 200)
                ),
                code,
            });
        }

        let mut opts = RequestOptions::default();
        opts.credential = Some(target);
        let reply = self
            .base
            .cgi_reply(
                "music.UserInfo.userInfoServer",
                "GetLoginUserInfo",
                json!({}),
                opts,
            )
            .await?;
        // 仅把凭证过期码 (1000/104401/104400) 判为过期;
        // 其他业务码 (如 2001 限流、10007 等) 不是"过期", 应透传为错误,
        // 避免上层误判后触发更多请求.
        match reply.code {
            0 => Ok(false),
            1000 | 104401 | 104400 => Ok(true),
            other => Err(QmError::CgiApi {
                code: other,
                data: crate::error::redact_payload(&reply.data.to_string(), 200),
            }),
        }
    }

    /// 尝试刷新登录凭证.
    pub async fn refresh_credential(&self, credential: Option<&Credential>) -> Result<Credential> {
        let target = credential
            .cloned()
            .unwrap_or_else(|| self.base.credential());
        let param = match target.login_type {
            1 => json!({
                "openid": target.openid,
                "refresh_token": target.refresh_token,
                "str_musicid": target.str_musicid(),
                "musickey": target.musickey,
                "unionid": target.unionid,
                "refresh_key": target.refresh_key,
                "loginMode": 2,
            }),
            2 => json!({
                "openid": target.openid,
                "access_token": target.access_token,
                "refresh_token": target.refresh_token,
                "expired_in": target.expired_at,
                "musicid": target.musicid,
                "musickey": target.musickey,
                "refresh_key": target.refresh_key,
                "loginMode": 2,
            }),
            _ => json!({
                "openid": target.openid,
                "access_token": target.access_token,
                "refresh_token": target.refresh_token,
                "expired_in": target.expired_at,
                "str_musicid": target.str_musicid(),
                "musicid": target.musicid,
                "musickey": target.musickey,
                "unionid": target.unionid,
                "refresh_key": target.refresh_key,
                "loginMode": 2,
            }),
        };
        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "tmeLoginType": target.login_type }));
        opts.credential = Some(target);
        let reply = self
            .base
            .cgi_reply("music.login.LoginServer", "Login", param, opts)
            .await?;
        Self::validate_result(reply).map_err(|e| match e {
            QmError::Login { message, code } => {
                QmError::CredentialRefresh(format!("{message} (code {code})"))
            }
            other => other,
        })
    }

    /// 登出当前账号.
    pub async fn logout(&self, credential: Option<&Credential>) -> Result<()> {
        let target = credential
            .cloned()
            .unwrap_or_else(|| self.base.credential());
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = Some(target.clone());
        let reply = self
            .base
            .cgi_reply("music.login.LoginServer", "Logout", json!({}), opts)
            .await?;
        // transport 成功 ≠ 业务成功: Logout 有本地 side effect, 必须确认业务码.
        if reply.code != 0 {
            return Err(QmError::Login {
                message: format!(
                    "登出失败: {}",
                    crate::error::redact_payload(&reply.data.to_string(), 200)
                ),
                code: reply.code,
            });
        }
        // 业务成功后清除本地状态.
        if credential.is_none() {
            self.base.context.set_credential(Credential::default());
        }
        // 使该账号的 Android session 失效 (旧鉴权状态下申请的 session 不再可用).
        self.base.context.invalidate_session(target.musicid).await;
        Ok(())
    }

    /// 获取指定类型的登录二维码.
    pub async fn get_qrcode(&self, login_type: QRLoginType) -> Result<QR> {
        match login_type {
            QRLoginType::Wx => self.get_wx_qr().await,
            QRLoginType::Mobile => self.get_mobile_qr().await,
            QRLoginType::Qq => self.get_qq_qr().await,
        }
    }

    /// 获取 QQ 授权二维码.
    pub async fn get_qq_qr(&self) -> Result<QR> {
        let url = "https://ssl.ptlogin2.qq.com/ptqrshow";
        let params: Vec<(String, String)> = vec![
            ("appid".to_string(), "716027609".to_string()),
            ("e".to_string(), "2".to_string()),
            ("l".to_string(), "M".to_string()),
            ("s".to_string(), "3".to_string()),
            ("d".to_string(), "72".to_string()),
            ("v".to_string(), "4".to_string()),
            ("t".to_string(), format!("{}", now_ms() as f64 / 1000.0)),
            ("daid".to_string(), "383".to_string()),
            ("pt_3rd_aid".to_string(), "100497308".to_string()),
        ];
        let mut opts = crate::client::HttpOptions::default();
        opts.headers.insert(
            "Referer",
            reqwest::header::HeaderValue::from_static("https://xui.ptlogin2.qq.com/"),
        );
        opts.params = params;

        // 直接使用 context 的 HTTP 客户端 (共享代理/限流/cookie).
        // 使用 reqwest `.query()` 进行 URL 编码, 避免参数含保留字符出错.
        self.base.context.limiter.acquire().await;
        let resp = self
            .base
            .context
            .http
            .get(url)
            .query(&opts.params)
            .headers(opts.headers)
            .send()
            .await
            .map_err(QmError::from)?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(QmError::http(status, String::new()));
        }
        let qrsig = extract_cookie(&resp, "qrsig");
        if qrsig.is_empty() {
            return Err(QmError::ApiData("获取 qrsig 失败".into()));
        }
        let bytes = resp.bytes().await.map_err(QmError::from)?;
        Ok(QR {
            data: bytes.to_vec(),
            qr_type: QRLoginType::Qq,
            mimetype: "image/png".into(),
            identifier: qrsig,
        })
    }

    /// 获取微信登录二维码.
    pub async fn get_wx_qr(&self) -> Result<QR> {
        let params = vec![
            ("appid", "wx48db31d50e334801"),
            (
                "redirect_uri",
                "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/",
            ),
            ("response_type", "code"),
            ("scope", "snsapi_login"),
            ("state", "STATE"),
            (
                "href",
                "https://y.qq.com/mediastyle/music_v17/src/css/popup_wechat.css#wechat_redirect",
            ),
        ];
        let opts = crate::client::HttpOptions {
            params: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        };
        let text = self
            .base
            .context
            .request_http(
                reqwest::Method::GET,
                "https://open.weixin.qq.com/connect/qrconnect",
                &opts,
            )
            .await?;
        // 提取 uuid="..."
        let uuid = extract_between(&text, "uuid=", "\"");
        let uuid = uuid.ok_or_else(|| QmError::ApiData("获取 uuid 失败".into()))?;
        let opts = crate::client::HttpOptions {
            headers: {
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(
                    "Referer",
                    reqwest::header::HeaderValue::from_static(
                        "https://open.weixin.qq.com/connect/qrconnect",
                    ),
                );
                h
            },
            ..Default::default()
        };
        let text = self
            .base
            .context
            .request_http(
                reqwest::Method::GET,
                &format!("https://open.weixin.qq.com/connect/qrcode/{uuid}"),
                &opts,
            )
            .await?;
        Ok(QR {
            data: text.into_bytes(),
            qr_type: QRLoginType::Wx,
            mimetype: "image/jpeg".into(),
            identifier: uuid,
        })
    }

    /// 获取手机客户端登录二维码 (状态检查需要 MQTT, 参见文档).
    pub async fn get_mobile_qr(&self) -> Result<QR> {
        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "ct": 23, "cv": 0 }));
        let mut param = json!({ "tmeAppID": "qqmusic" });
        let version_params = self.base.build_version_params(None);
        if let Value::Object(map) = version_params {
            for (k, v) in map {
                param[k] = v;
            }
        }
        let data = self
            .base
            .cgi("music.login.LoginServer", "CreateQRCode", param, opts)
            .await?;
        let qrcode = data.get("qrcode").and_then(Value::as_str).unwrap_or("");
        let qrcode_id = data.get("qrcodeID").and_then(Value::as_str).unwrap_or("");
        if qrcode.is_empty() || qrcode_id.is_empty() {
            return Err(QmError::ApiData("获取二维码失败".into()));
        }
        // base64 data URL, 如 "data:image/png;base64,xxxx".
        let b64_part = qrcode.split(',').next_back().unwrap_or("");
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_part)
            .map_err(|e| QmError::ApiData(format!("解码二维码失败: {e}")))?;
        Ok(QR {
            data: bytes,
            qr_type: QRLoginType::Mobile,
            mimetype: "image/png".into(),
            identifier: qrcode_id.to_string(),
        })
    }

    /// 检查二维码登录状态 (QQ / 微信).
    pub async fn check_qrcode(&self, qrcode: &QR) -> Result<QRLoginResult> {
        match qrcode.qr_type {
            QRLoginType::Wx => self.check_wx_qr(qrcode).await,
            QRLoginType::Mobile => Err(QmError::ApiData(
                "手机客户端二维码状态需要 MQTT, 请使用 checking_mobile_qrcode".into(),
            )),
            QRLoginType::Qq => self.check_qq_qr(qrcode).await,
        }
    }

    /// 检查手机客户端二维码登录状态 (基于 MQTT 推送).
    ///
    /// 建立 MQTT 连接并订阅 `management.qrcode_login/{qrcode_id}`,
    /// 持续接收服务端推送的登录状态事件. 到达终端事件
    /// (DONE / REFUSE / TIMEOUT) 时停止.
    ///
    /// `timeout` 是**整个二维码生命周期**的总时限, 不会因为中间收到非终止
    /// 消息而重新计时. 连接期间按 MQTT Keep Alive (含服务端 override) 发送 PINGREQ.
    ///
    /// Args:
    ///     qrcode: 由 `get_qrcode(QRLoginType::Mobile)` 获取的二维码对象.
    ///     timeout: 整体登录超时; 超时产出 `TIMEOUT` 事件后结束.
    pub async fn checking_mobile_qrcode(
        &self,
        qrcode: &QR,
        timeout: std::time::Duration,
    ) -> Result<Vec<QRLoginResult>> {
        use crate::mqtt::{keep_alive_interval, MqttClient, MqttProperties};
        use rand::Rng;
        use std::time::Instant;

        let qrcode_id = qrcode.identifier.clone();
        let now_ms = now_ms();
        let rand_part: i64 = rand::thread_rng().gen_range(1000..9999);
        let client_id = format!("{now_ms}{rand_part}");

        let connect_props = MqttProperties::default()
            .auth_method("pass")
            .user_property(&[
                ("tmeAppID", "qqmusic"),
                ("business", "management"),
                ("hashTag", &qrcode_id),
                ("clientTag", "management.user"),
                ("userID", &qrcode_id),
            ]);
        let headers = vec![
            ("Origin".to_string(), "https://y.qq.com".to_string()),
            ("Referer".to_string(), "https://y.qq.com/".to_string()),
            (
                "User-Agent".to_string(),
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36"
                    .to_string(),
            ),
        ];

        let mut client = MqttClient::connect(
            "mu.y.qq.com",
            443,
            "/ws/handshake",
            &client_id,
            45,
            &connect_props,
            &headers,
        )
        .await
        .map_err(|e| QmError::network(format!("MQTT 连接失败: {e}")))?;

        let topic = format!("management.qrcode_login/{qrcode_id}");
        let sub_props = MqttProperties::default()
            .user_property(&[("authorization", "tmelogin"), ("pubsub", "unicast")]);
        client.subscribe(&topic, &sub_props).await?;

        let mut events = vec![QRLoginResult {
            event: QRCodeLoginEvents::Scan,
            credential: None,
        }];

        let deadline = Instant::now() + timeout;
        let ping_period = keep_alive_interval(client.keep_alive);
        let mut ping = match ping_period {
            Some(period) => {
                let mut iv = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                Some(iv)
            }
            None => None,
        };

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                events.push(QRLoginResult {
                    event: QRCodeLoginEvents::Timeout,
                    credential: None,
                });
                break;
            }

            let timed_out = if let Some(iv) = ping.as_mut() {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep(remaining) => true,
                    msg = client.next_message() => {
                        match msg {
                            Ok(message) => {
                                if self
                                    .ingest_mobile_mqtt(&qrcode_id, message, &mut events)
                                    .await?
                                {
                                    break;
                                }
                            }
                            Err(e) => return Err(e),
                        }
                        false
                    }
                    _ = iv.tick() => {
                        if let Err(e) = client.ping().await {
                            if !e.is_retryable() {
                                return Err(e);
                            }
                        }
                        false
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep(remaining) => true,
                    msg = client.next_message() => {
                        match msg {
                            Ok(message) => {
                                if self
                                    .ingest_mobile_mqtt(&qrcode_id, message, &mut events)
                                    .await?
                                {
                                    break;
                                }
                            }
                            Err(e) => return Err(e),
                        }
                        false
                    }
                }
            };
            if timed_out {
                events.push(QRLoginResult {
                    event: QRCodeLoginEvents::Timeout,
                    credential: None,
                });
                break;
            }
        }
        Ok(events)
    }

    /// 消化一条 MQTT 推送. 返回 `true` 表示已到达终端事件.
    async fn ingest_mobile_mqtt(
        &self,
        qrcode_id: &str,
        message: crate::mqtt::MqttMessage,
        events: &mut Vec<QRLoginResult>,
    ) -> Result<bool> {
        let message_type = message.properties.get("type").cloned();
        let payload = message.json();
        let item = self
            .handle_mobile_message(qrcode_id, message_type.as_deref(), payload)
            .await?;
        if let Some(item) = item {
            let terminal = matches!(
                item.event,
                QRCodeLoginEvents::Done | QRCodeLoginEvents::Refuse | QRCodeLoginEvents::Timeout
            );
            events.push(item);
            return Ok(terminal);
        }
        Ok(false)
    }

    /// 处理手机客户端登录事件消息.
    async fn handle_mobile_message(
        &self,
        qrcode_id: &str,
        event_type: Option<&str>,
        payload: Option<Value>,
    ) -> Result<Option<QRLoginResult>> {
        match event_type {
            Some("scanned") => Ok(Some(QRLoginResult {
                event: QRCodeLoginEvents::Conf,
                credential: None,
            })),
            Some("canceled") => Ok(Some(QRLoginResult {
                event: QRCodeLoginEvents::Refuse,
                credential: None,
            })),
            Some("timeout") => Ok(Some(QRLoginResult {
                event: QRCodeLoginEvents::Timeout,
                credential: None,
            })),
            Some("loginFailed") => Err(QmError::Login {
                message: "登录失败".into(),
                code: -1,
            }),
            Some("cookies") => {
                let payload =
                    payload.ok_or_else(|| QmError::ApiData("无效的 MQTT 消息格式".into()))?;
                let cookies = &payload["cookies"];
                let uin = cookies["qqmusic_uin"]["value"].as_str().unwrap_or("");
                let key = cookies["qqmusic_key"]["value"].as_str().unwrap_or("");
                if uin.is_empty() || key.is_empty() {
                    return Err(QmError::ApiData("获取登录凭据失败: 缺少必要参数".into()));
                }
                let mut opts = RequestOptions::default();
                opts.comm = Some(json!({ "tmeLoginType": 6 }));
                let reply = self
                    .base
                    .cgi_reply(
                        "music.login.LoginServer",
                        "Login",
                        json!({
                            "musicid": uin.parse::<i64>().unwrap_or(0),
                            "qrCodeID": qrcode_id,
                            "token": key,
                        }),
                        opts,
                    )
                    .await?;
                let credential = Self::validate_result(reply)?;
                Ok(Some(QRLoginResult {
                    event: QRCodeLoginEvents::Done,
                    credential: Some(credential),
                }))
            }
            _ => Ok(None),
        }
    }

    async fn check_qq_qr(&self, qrcode: &QR) -> Result<QRLoginResult> {
        let qrsig = qrcode.identifier.clone();
        let ptqrtoken = hash33(&qrsig, 0);
        let ptqrtoken_str = ptqrtoken.to_string();
        let action = format!("0-0-{}", now_ms());
        let params: Vec<(String, String)> = vec![
            (
                "u1".to_string(),
                "https://graph.qq.com/oauth2.0/login_jump".to_string(),
            ),
            ("ptqrtoken".to_string(), ptqrtoken_str),
            ("ptredirect".to_string(), "0".to_string()),
            ("h".to_string(), "1".to_string()),
            ("t".to_string(), "1".to_string()),
            ("g".to_string(), "1".to_string()),
            ("from_ui".to_string(), "1".to_string()),
            ("ptlang".to_string(), "2052".to_string()),
            ("action".to_string(), action),
            ("js_ver".to_string(), "20102616".to_string()),
            ("js_type".to_string(), "1".to_string()),
            ("pt_uistyle".to_string(), "40".to_string()),
            ("aid".to_string(), "716027609".to_string()),
            ("daid".to_string(), "383".to_string()),
            ("pt_3rd_aid".to_string(), "100497308".to_string()),
            ("has_onekey".to_string(), "1".to_string()),
        ];
        let mut opts = crate::client::HttpOptions::default();
        opts.headers.insert(
            "Referer",
            reqwest::header::HeaderValue::from_static("https://xui.ptlogin2.qq.com/"),
        );
        opts.cookies = vec![("qrsig".to_string(), qrsig)];
        opts.params = params;
        let text = self
            .base
            .context
            .request_http(
                reqwest::Method::GET,
                "https://ssl.ptlogin2.qq.com/ptqrlogin",
                &opts,
            )
            .await?;

        let body = extract_between(&text, "ptuiCB(", ")").unwrap_or_default();
        let args = extract_quoted(body.as_str());
        if args.is_empty() {
            return Err(QmError::ApiData("获取二维码状态失败: 无法解析响应".into()));
        }
        let code = args[0]
            .parse::<i64>()
            .map_err(|_| QmError::ApiData("无效的状态码".into()))?;
        let event = QRCodeLoginEvents::get_by_value(code)
            .ok_or_else(|| QmError::ApiData(format!("无法识别的状态码: {code}")))?;
        if event != QRCodeLoginEvents::Done {
            return Ok(QRLoginResult {
                event,
                credential: None,
            });
        }
        if args.len() < 3 {
            return Err(QmError::ApiData("获取登录凭据失败: 缺少必要参数".into()));
        }
        let sigx = extract_between(&args[2], "ptsigx=", "&s_url").unwrap_or_default();
        let uin = extract_between(&args[2], "uin=", "&service").unwrap_or_default();
        if sigx.is_empty() || uin.is_empty() {
            return Err(QmError::ApiData(
                "获取登录凭据失败: 无法解析必要参数".into(),
            ));
        }
        let credential = self.authorize_qq_qr(&uin, &sigx).await?;
        Ok(QRLoginResult {
            event,
            credential: Some(credential),
        })
    }

    async fn check_wx_qr(&self, qrcode: &QR) -> Result<QRLoginResult> {
        let uuid = qrcode.identifier.clone();
        let opts = crate::client::HttpOptions {
            params: vec![
                ("uuid".to_string(), uuid.clone()),
                ("_".to_string(), now_ms().to_string()),
            ],
            headers: {
                let mut h = reqwest::header::HeaderMap::new();
                h.insert(
                    "Referer",
                    reqwest::header::HeaderValue::from_static("https://open.weixin.qq.com/"),
                );
                h
            },
            timeout: Some(std::time::Duration::from_secs(35)),
            ..Default::default()
        };
        let text = self
            .base
            .context
            .request_http(
                reqwest::Method::GET,
                "https://lp.open.weixin.qq.com/connect/l/qrconnect",
                &opts,
            )
            .await
            .map_err(|e| match e {
                // 微信长轮询到点返回超时是正常"暂无结果"信号, 其余网络错误
                // (DNS/连接失败等) 保持分类, 不吞成 timeout.
                QmError::Network(n) if n.kind == crate::error::NetworkErrorKind::Timeout => {
                    QmError::Other("timeout".into())
                }
                other => other,
            })?;

        // 形如 window.wx_errcode=408;window.wx_code=''
        let errcode = extract_between(&text, "window.wx_errcode=", ";");
        let wx_code = extract_between(&text, "window.wx_code='", "'");
        let code = errcode
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| QmError::ApiData("获取二维码状态失败: 无法解析响应".into()))?;
        let event = QRCodeLoginEvents::get_by_value(code)
            .ok_or_else(|| QmError::ApiData(format!("无法识别的状态码: {code}")))?;
        if event != QRCodeLoginEvents::Done {
            return Ok(QRLoginResult {
                event,
                credential: None,
            });
        }
        let wx_code = wx_code.ok_or_else(|| QmError::ApiData("获取 code 失败".into()))?;
        let credential = self.authorize_wx_qr(&wx_code).await?;
        Ok(QRLoginResult {
            event,
            credential: Some(credential),
        })
    }

    /// 完成 QQ 二维码鉴权并返回凭证.
    async fn authorize_qq_qr(&self, uin: &str, sigx: &str) -> Result<Credential> {
        let params = vec![
            ("uin", uin),
            ("pttype", "1"),
            ("service", "ptqrlogin"),
            ("nodirect", "0"),
            ("ptsigx", sigx),
            ("s_url", "https://graph.qq.com/oauth2.0/login_jump"),
            ("ptlang", "2052"),
            ("ptredirect", "100"),
            ("aid", "716027609"),
            ("daid", "383"),
            ("j_later", "0"),
            ("low_login_hour", "0"),
            ("regmaster", "0"),
            ("pt_login_type", "3"),
            ("pt_aid", "0"),
            ("pt_aaid", "16"),
            ("pt_light", "0"),
            ("pt_3rd_aid", "100497308"),
        ];
        let mut opts = crate::client::HttpOptions::default();
        opts.headers.insert(
            "Referer",
            reqwest::header::HeaderValue::from_static("https://xui.ptlogin2.qq.com/"),
        );
        opts.params = params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        // 使用 reqwest `.query()` 进行正确的 URL 编码 (ptsigx 等含保留字符).
        self.base.context.limiter.acquire().await;
        let resp = self
            .base
            .context
            .http
            .get("https://ssl.ptlogin2.graph.qq.com/check_sig")
            .query(&opts.params)
            .headers(opts.headers)
            .send()
            .await
            .map_err(QmError::from)?;
        let p_skey = extract_cookie(&resp, "p_skey");
        if p_skey.is_empty() {
            return Err(QmError::ApiData("获取 p_skey 失败".into()));
        }

        // 构造 authorize 表单请求
        let data = json!({
            "response_type": "code",
            "client_id": "100497308",
            "redirect_uri": "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/",
            "scope": "get_user_info,get_app_friends",
            "state": "state",
            "switch": "",
            "from_ptlogin": "1",
            "src": "1",
            "update_auth": "1",
            "openapi": "1010_1030",
            "g_tk": hash33(&p_skey, 5381),
            "auth_time": now_ms().to_string(),
        });
        // 使用 context 的 HTTP 客户端 (共享代理/限流/cookie), 跟随重定向后
        // 从最终 URL 提取授权码 (OAuth 跳转到 redirect_uri?code=...).
        self.base.context.limiter.acquire().await;
        let resp = self
            .base
            .context
            .http
            .post("https://graph.qq.com/oauth2.0/authorize")
            .header("Referer", "https://xui.ptlogin2.qq.com/")
            .header("Cookie", format!("uin={uin}; p_skey={p_skey}"))
            .form(&data)
            .send()
            .await
            .map_err(QmError::from)?;
        let status = resp.status().as_u16();
        let final_url = resp.url().clone();
        let code = final_url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        if status != 200 || code.is_empty() {
            return Err(QmError::ApiData("获取 code 失败".into()));
        }

        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "tmeLoginType": 2 }));
        let reply = self
            .base
            .cgi_reply(
                "QQConnectLogin.LoginServer",
                "QQLogin",
                json!({ "code": code }),
                opts,
            )
            .await?;
        Self::validate_result(reply)
    }

    /// 完成微信二维码鉴权并返回凭证.
    async fn authorize_wx_qr(&self, code: &str) -> Result<Credential> {
        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "tmeLoginType": 1 }));
        let reply = self
            .base
            .cgi_reply(
                "music.login.LoginServer",
                "Login",
                json!({ "code": code, "strAppid": "wx48db31d50e334801" }),
                opts,
            )
            .await?;
        Self::validate_result(reply)
    }

    /// 发送手机验证码.
    pub async fn send_authcode(
        &self,
        phone: &str,
        is_encrypted: bool,
        country_code: i64,
    ) -> Result<PhoneAuthCodeResult> {
        let mut param = json!({ "tmeAppid": "qqmusic", "areaCode": country_code.to_string() });
        if is_encrypted {
            param["encryptedPhoneNo"] = json!(phone);
        } else {
            param["phoneNo"] = json!(phone);
        }
        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "tmeLoginMethod": 3 }));
        opts.platform = Some(Platform::Android);
        let reply = self
            .base
            .cgi_reply("music.login.LoginServer", "SendPhoneAuthCode", param, opts)
            .await?;
        match reply.code {
            20276 => Ok(PhoneAuthCodeResult {
                event: PhoneLoginEvents::Captcha,
                info: reply
                    .data
                    .get("securityURL")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string()),
            }),
            100001 => Ok(PhoneAuthCodeResult {
                event: PhoneLoginEvents::Frequency,
                info: None,
            }),
            0 => Ok(PhoneAuthCodeResult {
                event: PhoneLoginEvents::Send,
                info: None,
            }),
            _ => Err(QmError::Login {
                message: "发送验证码失败".into(),
                code: reply.code,
            }),
        }
    }

    /// 使用手机验证码鉴权.
    pub async fn phone_authorize(
        &self,
        phone: &str,
        is_encrypted: bool,
        auth_code: &str,
    ) -> Result<Credential> {
        let mut param = json!({ "code": auth_code, "loginMode": 1 });
        if is_encrypted {
            param["encryptedPhoneNo"] = json!(phone);
        } else {
            param["phoneNo"] = json!(phone);
        }
        let mut opts = RequestOptions::default();
        opts.comm = Some(json!({ "tmeLoginMethod": 3, "tmeLoginType": 0 }));
        opts.platform = Some(Platform::Android);
        let reply = self
            .base
            .cgi_reply("music.login.LoginServer", "Login", param, opts)
            .await?;
        Self::validate_result(reply)
    }
}

/// 从字符串中提取两个标记之间的内容.
fn extract_between(text: &str, start: &str, end: &str) -> Option<String> {
    let idx = text.find(start)?;
    let rest = &text[idx + start.len()..];
    let end_idx = rest.find(end)?;
    Some(rest[..end_idx].to_string())
}

/// 从响应头中提取指定 Cookie 值.
fn extract_cookie(resp: &reqwest::Response, name: &str) -> String {
    let mut value = String::new();
    for header in resp.headers().get_all("set-cookie") {
        if let Ok(s) = header.to_str() {
            if let Some(part) = s.split(';').next() {
                let part = part.trim();
                if let Some((k, v)) = part.split_once('=') {
                    if k.trim() == name {
                        value = v.to_string();
                    }
                }
            }
        }
    }
    value
}
