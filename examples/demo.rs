//! 快速演示: 使用 qqmusic-api 检索歌曲、获取播放链接与歌词.
//!
//! 运行: `cargo run --example demo`

use qqmusic_api::modules::song::{SongFileInfo, SongFileType};
use qqmusic_api::{Client, SearchType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(None, None)?;

    // 1. 类型搜索
    let resp = client
        .search
        .search_by_type("周杰伦", SearchType::Song, 5, 1, &[], None, true)
        .await?;
    println!("搜索「周杰伦」命中 {} 条", resp.song.len());
    let first = resp.song.first().cloned();
    let Some(song) = first else {
        println!("未搜索到结果");
        return Ok(());
    };
    let song = song.base;
    println!(
        "歌曲: {} - {} (mid={})",
        song.name,
        song.singer
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(" / "),
        song.mid
    );

    // 2. 获取标准音质播放链接 (无需登录)
    let urls = client
        .song
        .get_song_urls(
            &[SongFileInfo::new(&song.mid).with_type(SongFileType::Mp3_128)],
            &SongFileType::Mp3_128,
            None,
        )
        .await?;
    for u in &urls.data {
        if !u.purl.is_empty() {
            println!("播放链接: {}{}", song.mid, u.purl);
        }
    }

    // 3. 获取歌词
    match client
        .lyric
        .get_lyric(&song.mid, 1, false, false, false, false)
        .await
    {
        Ok(lyric) => println!(
            "歌词(前100字): {}",
            lyric.lyric.chars().take(100).collect::<String>()
        ),
        Err(e) => println!("获取歌词失败: {e}"),
    }

    Ok(())
}
