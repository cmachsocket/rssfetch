use futures;
use html2text;
use reqwest;
use rss::Channel;
use serde::Deserialize;
use serde_saphyr;
use std::error::Error;
use std::fs::OpenOptions;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path};
use std::{format, writeln};
use chrono::{Local, Utc};
use chrono::DateTime;
use log::{debug, error, info, warn};

/// 初始化日志后端。log 只是一个门面，必须注册一个实现才能真正输出。
/// 默认级别 info，可用 RUST_LOG=debug 环境变量调高。
fn init_logger() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .format_target(false)
        .init();
}

async fn feed(url: &str) -> Result<Channel, Box<dyn Error>> {
    // 错误分支 1: 网络层（DNS/连接/超时/TLS）
    let resp = reqwest::get(url)
        .await
        .map_err(|e| format!("HTTP 请求失败 [{url}]: {e}"))?;

    // 错误分支 2: HTTP 状态码非 2xx。reqwest::get 不会自动报错，
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
    let channel = Channel::read_from(&content[..])
        .map_err(|e| format!("解析 RSS 失败 [{url}]: {e}"))?;

    debug!("拉取成功 [{url}]: {} 个条目", channel.items().len());
    Ok(channel)
}

/// 写入一行；失败只记日志并继续，不中断整个订阅源。
macro_rules! write_line {
    ($w:expr, $route:expr, $field:expr, $($arg:tt)*) => {
        match writeln!($w, $($arg)*) {
            Ok(()) => {}
            Err(e) => log::error!("写入文件失败 (路由 {}, 字段 {}): {}", $route, $field, e),
        }
    };
}

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "localhost")]
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "String::new")]
    access_key: String,
    #[serde(default = "String::new")]
    save_path: String,
    #[serde(default = "Vec::new")]
    routes: Vec<String>,
}

fn localhost() -> String {
    "localhost".into()
}
fn default_port() -> u16 {
    80
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    init_logger();
    info!("rssfetch 启动");

    // 把所有致命错误统一经由 log 输出（带级别和时间戳），
    // 同时保留非 0 退出码，便于 crontab / CI 感知失败。
    if let Err(e) = run().await {
        error!("致命错误: {}", e);
        return Err(e);
    }
    info!("rssfetch 结束");
    Ok(())
}

async fn run() -> Result<(), Box<dyn Error>> {
    // 错误分支 5: 配置文件不存在 / 无读权限
    let file = std::fs::File::open("config.yml")
        .map_err(|e| format!("打开配置文件 config.yml 失败: {e}"))?;

    // 错误分支 6: YAML 语法错误，或字段类型不匹配
    let config: Config = serde_saphyr::from_reader(file)
        .map_err(|e| format!("解析 config.yml 失败: {e}"))?;

    let host = config.host;
    let port =  config.port;
    let access_key = config.access_key;
    let save_path = config.save_path;
    let routes = config.routes;

    if routes.is_empty() {
        warn!("config.yml 中没有配置任何 routes，程序无事可做");
        return Ok(());
    }
    info!("共 {} 个订阅源, host={}:{}", routes.len(), host, port);

    // 错误分支 7: 创建输出目录失败（路径是文件、无权限等）
    if save_path.is_empty() {
        warn!("save_path 为空, 将直接写入当前目录");
    } else if let Err(e) = fs::create_dir_all(&save_path) {
        // 不在此处重复 error!，由 main 的"致命错误"统一输出，避免同一错误打印两遍
        return Err(format!("创建输出目录 [{}] 失败: {e}", save_path).into());
    }

    let now = Local::now();
    let path = Path::new(&save_path).join(format!("{}.txt", now.format("%Y-%m-%d")));

    // 错误分支 8: 打不开输出文件（已存在但无写权限等）
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开输出文件 [{}] 失败: {}", path.display(), e))?;
    info!("输出文件: {}", path.display());

    let results = futures::future::join_all(routes.iter().map(|route| {
        let host = host.clone();
        let port = port;
        let access_key = access_key.clone();
        // 错误分支 9: 克隆文件句柄失败。原代码用 expect 直接 panic，
        // 这里改成返回错误，交给下面的结果汇总统一记录。
        let f = f
            .try_clone()
            .map_err(|e| format!("克隆文件句柄失败 [{route}]: {e}"));
        async move {
            let mut w = BufWriter::new(f?);
            let url = format!("http://{host}:{port}{route}?key={access_key}");

            // 错误分支 10: 单个订阅源抓取/解析失败
            let channel = feed(&url).await.map_err(|e| format!("订阅源处理失败 [{route}]: {e}"))?;

            channel.items().iter().for_each(|item| {
                // 缺失字段属于正常情况，用占位符，不算错误
                let title = item.title().unwrap_or("(无标题)");
                let pub_date = item.pub_date().unwrap_or("(无发布时间)");

                // 错误分支 11: pub_date 不是合法 RFC2822（或缺失）。
                // 原代码 unwrap 到 epoch 且不记录，导致这类条目被当成 1970 年的
                // 旧条目静默丢弃。这里保留 epoch 兜底（过滤行为不变），
                // 但补一条 warn，让"条目丢失"变得可追溯。
                let dt = match DateTime::parse_from_rfc2822(pub_date) {
                    Ok(dt) => dt.into(),
                    Err(e) => {
                        warn!("发布时间无法解析 [{route}] 标题='{}' 值='{}': {}（该条目将按 1970 计入并被 1 天窗口过滤掉）", title, pub_date, e);
                        let epoch = DateTime::<Utc>::from_timestamp(0, 0)
                            .expect("epoch 时间戳恒为有效值");
                        epoch.with_timezone(&Local)
                    }
                };

                if dt < Local::now() - chrono::Duration::days(1) {
                    info!("Skipping item '{}' published on {} (older than 1 day)", title, pub_date);
                    return; // Skip items older than 1 day
                }

                println!("- 标题: {}", title);
                write_line!(w, route, "标题", "- 标题: {}", title);
                println!("  发布时间: {}", pub_date);
                write_line!(w, route, "发布时间", "  发布时间: {}", pub_date);

                // 错误分支 12/13: 描述与正文的处理
                item.description().inspect(|content| {
                    let description = match html2text::from_read(content.as_bytes(), content.len() * 3) {
                        Ok(text) => format!("  描述: {}", text),
                        Err(e) => {
                            // 原来 unwrap_or 静默吞掉，这里记录
                            warn!("HTML 转文本失败 [{route}] 标题='{}': {}", title, e);
                            format!("  描述: {}", content)
                        }
                    };
                    println!("{}", description);
                    write_line!(w, route, "描述", "{}", description);
                });
                item.content()
                    .inspect(|content| println!("  内容: {}", content));
            });

            // 错误分支 14: 缓冲区刷盘失败
            w.flush().map_err(|e| format!("刷盘失败 [{route}]: {e}"))?;
            info!("订阅源完成 [{route}]");
            Ok::<(), Box<dyn Error>>(())
        }
    }))
    .await;

    // 错误分支 15（关键）: 原代码 `join_all(...).await;` 把整个 Vec<Result> 直接丢弃，
    // 任何一个订阅源失败都毫无痕迹。这里逐个检查并汇总。
    let total = results.len();
    let mut failed = 0usize;
    for res in results {
        if let Err(e) = res {
            failed += 1;
            error!("{}", e);
        }
    }
    if failed == 0 {
        info!("全部 {total} 个订阅源处理成功");
    } else {
        return Err(format!("{failed}/{total} 个订阅源处理失败").into());
    }

    Ok(())
}
