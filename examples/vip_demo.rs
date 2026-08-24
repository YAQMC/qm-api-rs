//! VIP 账号演示: 高音质链接 / 下载解密 / VIP 状态检测.
//!
//! 需要提供你自己的测试账号凭证 (非破解, 仅使用你账号已购的 VIP 权益):
//!
//! ```bash
//! export QM_MUSICID=你的musicid
//! export QM_MUSICKEY=你的musickey
//! export QM_LOGIN_TYPE=2   # 1=微信, 2=QQ
//! cargo run --example vip_demo
//! ```

use qqmusic_api::{Client, Credential, SearchType};

fn load_credential() -> Option<Credential> {
    let musicid: i64 = std::env::var("QM_MUSICID").ok()?.parse().ok()?;
    let musickey = std::env::var("QM_MUSICKEY").ok()?;
    let login_type: i64 = std::env::var("QM_LOGIN_TYPE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    Some(Credential {
        musicid,
        str_musicid: musicid.to_string(),
        musickey,
        login_type,
        ..Default::default()
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential = load_credential()
        .ok_or_else(|| "请先设置环境变量 QM_MUSICID / QM_MUSICKEY (你的测试账号)".to_string())?;
    let client = Client::new(Some(credential.clone()), None)?;

    // 1. VIP 状态检测
    match client.user.is_vip(Some(&credential)).await {
        Ok(true) => println!("[VIP] 账号 {} 具备 VIP 权益", credential.musicid),
        Ok(false) => println!(
            "[非VIP] 账号 {} 无 VIP 权益 (部分高音质不可用)",
            credential.musicid
        ),
        Err(e) => println!("[VIP检查失败] {e} (凭证可能无效)"),
    }

    // 2. 搜索一首歌, 查看可用音质
    let resp = client
        .search
        .search_by_type("晴天", SearchType::Song, 1, 1, &[], None, true)
        .await?;
    let song = resp.song[0].base.clone();
    println!(
        "歌曲: {} - {}",
        song.name,
        song.singer
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("/")
    );

    let qualities = client.song.available_qualities(&song);
    println!(
        "可用音质: {:?}",
        qualities
            .iter()
            .map(|q| format!("{q:?}"))
            .collect::<Vec<_>>()
    );

    // 3. 获取最高音质播放链接 (加密音质走 CgiGetEVkey, 需要 VIP)
    match client
        .song
        .get_best_song_url(&song, Some(&credential), true)
        .await
    {
        Ok((quality, urls)) => {
            println!("最高音质: {quality:?}");
            for item in urls.data.iter().take(2) {
                println!(
                    "  filename={} result={} purl.len={} ekey.len={}",
                    item.filename,
                    item.result,
                    item.purl.len(),
                    item.ekey.len()
                );
            }
            // 4. 下载并解密
            match client
                .song
                .download_quality(&song, quality, Some(&credential))
                .await
            {
                Ok((audio, ext)) => {
                    println!("[下载解密成功] {} 字节, 格式 .{}", audio.len(), ext);
                    let out = std::path::Path::new("output");
                    std::fs::create_dir_all(out)?;
                    std::fs::write(out.join(format!("{}.{}", song.mid, ext)), &audio)?;
                    println!("已保存到 output/{}.{}", song.mid, ext);
                }
                Err(e) => println!("[下载失败] {e} (若无 VIP 权限会返回 104003)"),
            }
        }
        Err(e) => println!("[获取音质失败] {e}"),
    }

    Ok(())
}
