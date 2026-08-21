//! 第三方客户端能力演示: QMC 解密 / LRC 解析 / 凭证管理 / 代理.
//!
//! 运行: `cargo run --example client_ready`

use qqmusic_api::lyric_parser::{Lyric, QrcLyric};
use qqmusic_api::{Client, Credential, CredentialStore, SearchType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 创建客户端 (可指定代理)
    let client = Client::new_with_proxy(None, None, std::env::var("HTTP_PROXY").ok().as_deref())?;

    // 2. 搜索 + 获取歌词 (LRC)
    let resp = client
        .search
        .search_by_type("晴天", SearchType::Song, 1, 1, &[], None, true)
        .await?;
    let song = &resp.song[0].base;
    println!("歌曲: {} - {}", song.name, song.singer[0].name);

    let lyric = client
        .lyric
        .get_lyric(&song.mid, 1, false, false, false, false)
        .await?;
    let parsed = Lyric::parse(&lyric.lyric);
    println!("LRC 歌词行数: {}", parsed.lines.len());
    println!(
        "  第 3 秒处: {:?}",
        parsed.line_at(3000).map(|l| l.text.as_str())
    );

    // 3. QRC 逐字歌词
    if let Ok(qrc) = client
        .lyric
        .get_lyric(&song.mid, 1, true, false, false, false)
        .await
    {
        let parsed_qrc = QrcLyric::parse(&qrc.lyric);
        println!("QRC 逐字歌词行数: {}", parsed_qrc.lines.len());
    }

    // 4. 凭证管理 (持久化 + 自动刷新)
    let store_path = std::env::temp_dir().join("qqmusic_credentials.json");
    let store = match CredentialStore::load(&store_path) {
        Ok(s) => s,
        Err(_) => CredentialStore::new(),
    };
    let store = store.with_path(&store_path);
    // 登录成功后: store.add(credential); store.save()?;
    println!("已保存账号数: {}", store.account_ids().len());

    // 5. QMC 加密音质解密
    //    获取加密音质链接和 ekey, 下载 .mflac/.mgg 文件后:
    //    let (audio, ext) = qqmusic_api::qmc::decrypt_file(path, Some(&ekey))?;
    println!("QMC 解密: qqmusic_api::qmc::decrypt_file(path, ekey)");

    Ok(())
}

#[allow(dead_code)]
fn _credential_example() -> Credential {
    Credential {
        musicid: 12345678,
        str_musicid: "12345678".into(),
        musickey: "your-musickey".into(),
        login_type: 2,
        ..Default::default()
    }
}
