
mod get_rss;
mod llm_sum;
mod get_cfg;
use std::error::Error;
use log::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    get_rss::init_logger();
    info!("rssfetch 启动");
    // 把所有致命错误统一经由 log 输出（带级别和时间戳），
    // 同时保留非 0 退出码，便于 crontab / CI 感知失败。
    let config = get_cfg::RSSconfig::new()?;
    if let Err(e) = get_rss::run(&config).await {
        error!("致命错误: {}", e);
        return Err(e);
    }
    llm_sum::llm_sum(&config).await;
    
    info!("rssfetch 结束");
    Ok(())
}

