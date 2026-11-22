use async_trait::async_trait;
use qqmusic_api::{
    ApiTransport, Client, Credential, HttpBody, HttpMethod, Platform, Result, TransportRequest,
    TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingTransport(Mutex<Vec<Value>>);

#[async_trait]
impl ApiTransport for RecordingTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        assert_eq!(request.method, HttpMethod::Post);
        let HttpBody::Json(payload) = request.body else {
            panic!("expected JSON");
        };
        self.0.lock().unwrap().push(payload.clone());
        let method = payload["req_0"]["method"].as_str().unwrap();
        let data = match method {
            "QueryLevelDetailPage" => json!({
                "powerInfo":{"level":4,"currentValue":30,"isHugeVip":true},
                "todayLT":120,
                "equities":null
            }),
            "QueryHugevipLevelRule" => json!({
                "currentDuration":60,"nextLevelDuration":120,
                "rules":[{"level":1,"needDuration":60,"percent":"10%"}]
            }),
            "GetRank" => json!({
                "ranks":null,"myRankNo":2,"myPlayTime":300,"hasMore":0,"rankno":5
            }),
            "Like" => json!({"NOUSE":0}),
            "SetPrivacy" => json!({"status":1}),
            "GetSoundPowerEntry" => json!({
                "Title":"听歌勋章","MedalCnt":1,
                "MedalList":[{"PicURL":"https://example.test/medal.png","Scheme":"qqmusic://medal"}]
            }),
            "GetTaskModules" => json!({
                "retCode":0,"retMsg":"ok","taskModules":[{"id":"task"}],"actInfo":{}
            }),
            _ => panic!("unexpected method: {method}"),
        };
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: vec![],
            body: serde_json::to_vec(&json!({"code":0,"req_0":{"code":0,"data":data}})).unwrap(),
        })
    }
}

fn credential() -> Credential {
    Credential {
        musicid: 10001,
        str_musicid: "10001".into(),
        musickey: "synthetic-key".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn all_sound_power_endpoints_use_reference_modules_and_parameters() {
    let transport = Arc::new(RecordingTransport::default());
    let client =
        Client::new_with_transport(Some(credential()), Some(Platform::Web), transport.clone());

    let detail = client.sound_power.get_detail(None).await.unwrap();
    assert_eq!(detail.power_info.level, 4);
    assert_eq!(detail.today_listen_time, 120);
    assert!(detail.equities.is_empty());

    let rule = client.sound_power.get_hugevip_rule(None).await.unwrap();
    assert_eq!(rule.rules[0].need_duration, 60);

    let rank = client
        .sound_power
        .get_friend_rank(10, 20, "last", 5, None)
        .await
        .unwrap();
    assert_eq!(rank.my_rank_no, 2);
    assert!(rank.ranks.is_empty());

    let liked = client
        .sound_power
        .like_friend("friend", false, None)
        .await
        .unwrap();
    assert_eq!(liked.no_use, 0);

    let privacy = client.sound_power.set_rank_privacy(1, None).await.unwrap();
    assert_eq!(privacy.status, 1);

    let medal = client
        .sound_power
        .get_medal_entry("encrypted", None)
        .await
        .unwrap();
    assert_eq!(medal.medal_cnt, 1);
    assert_eq!(
        medal.medal_list[0].pic_url,
        "https://example.test/medal.png"
    );

    let tasks = client.sound_power.get_tasks(None).await.unwrap();
    assert_eq!(tasks.ret_msg, "ok");
    assert_eq!(tasks.task_modules.unwrap()[0]["id"], "task");

    let module_ids = [String::from("A"), String::from("B")];
    let custom_tasks = client
        .sound_power
        .get_tasks_with("custom-act", Some(&module_ids), None)
        .await
        .unwrap();
    assert_eq!(custom_tasks.ret_code, 0);

    let requests = transport.0.lock().unwrap();
    let expected = [
        (
            "music.soundPower.SoundPowerSvr",
            "QueryLevelDetailPage",
            json!({}),
        ),
        (
            "music.soundPower.SoundPowerSvr",
            "QueryHugevipLevelRule",
            json!({}),
        ),
        (
            "music.activeCenter.FriendRankSvr",
            "GetRank",
            json!({"rank_type":1,"offset":10,"limit":20,"last_uin":"last","rankno":5}),
        ),
        (
            "music.activeCenter.FriendRankSvr",
            "Like",
            json!({"rank_type":1,"uin":"friend","cancel":0}),
        ),
        (
            "music.activeCenter.FriendRankSvr",
            "SetPrivacy",
            json!({"status":1}),
        ),
        (
            "music.medalHall.MedalHallEntrySrv",
            "GetSoundPowerEntry",
            json!({"EncUin":"encrypted"}),
        ),
        (
            "music.activeCenter.ActTaskNewSvr",
            "GetTaskModules",
            json!({"actID":"1nsAQf","taskModuleIDs":["Z1jtHy7"]}),
        ),
        (
            "music.activeCenter.ActTaskNewSvr",
            "GetTaskModules",
            json!({"actID":"custom-act","taskModuleIDs":["A","B"]}),
        ),
    ];
    assert_eq!(requests.len(), expected.len());
    for (index, (module, method, param)) in expected.into_iter().enumerate() {
        assert_eq!(requests[index]["req_0"]["module"], module);
        assert_eq!(requests[index]["req_0"]["method"], method);
        assert_eq!(requests[index]["req_0"]["param"], param);
    }
}

#[tokio::test]
async fn invalid_sound_power_inputs_never_send() {
    let transport = Arc::new(RecordingTransport::default());
    let client =
        Client::new_with_transport(Some(credential()), Some(Platform::Web), transport.clone());
    assert!(client
        .sound_power
        .get_friend_rank(-1, 10, "", 0, None)
        .await
        .is_err());
    assert!(client
        .sound_power
        .like_friend(" ", false, None)
        .await
        .is_err());
    assert!(client
        .sound_power
        .get_tasks_with("", None, None)
        .await
        .is_err());
    assert!(transport.0.lock().unwrap().is_empty());
}
