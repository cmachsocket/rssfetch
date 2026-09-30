use crate::FeedItem;
use chrono::DateTime;
use rss::{Channel, Item};
use std::time::Duration;

/// 拉取并解析单个订阅源。
///
/// 这里只做「取回来 + 解析成结构」，时间窗口过滤、markdown 拼装、文件写入
/// 都交给 Python 侧，方便在那里调整格式而不必重新编译 Rust。
pub(crate) async fn feed(url: &str, timeout: Duration) -> Result<Vec<FeedItem>, String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("rssfetch/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    // 错误分支 1: 网络层（DNS/连接/超时/TLS）
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP 请求失败 [{url}]: {e}"))?;

    // 错误分支 2: HTTP 状态码非 2xx。客户端不会自动报错，
    // 缺失这一步的话 404 的 HTML 会被当成 RSS 解析，报出难以定位的 XML 错误。
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("HTTP 状态异常 [{url}]: {e}"))?;

    // 错误分支 3: 读取响应体中断（连接被重置等）
    let content = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应体失败 [{url}]: {e}"))?;

    // 错误分支 4: XML 格式非法 / 不是 RSS
    let channel =
        Channel::read_from(&content[..]).map_err(|e| format!("解析 RSS 失败 [{url}]: {e}"))?;

    Ok(channel.items().iter().map(convert_item).collect())
}

/// `rss::Item` → 对外的 `FeedItem`。
fn convert_item(item: &Item) -> FeedItem {
    let pub_date = item.pub_date().map(str::to_owned);
    // pub_date 缺失或不是合法 RFC2822 时 published 为 None，
    // 由 Python 侧记 warn 后过滤掉，避免「条目静默消失」无法追溯。
    let published = pub_date
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc2822(value).ok())
        .map(|dt| dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_millis()) / 1000.0);

    FeedItem {
        // 缺失标题属于正常情况，用占位符，不算错误
        title: item.title().unwrap_or("(无标题)").to_owned(),
        link: item.link().map(str::to_owned),
        pub_date,
        published,
        description: item.description().map(str::to_owned),
        content: item.content().map(str::to_owned),
    }
}

/// HTML → 纯文本。`width` 是折行宽度（字符数），也用作读取缓冲区的上限提示。
pub(crate) fn html_to_text(html: &str, width: usize) -> Result<String, String> {
    html2text::from_read(html.as_bytes(), width).map_err(|e| format!("HTML 转文本失败: {e}"))
}
