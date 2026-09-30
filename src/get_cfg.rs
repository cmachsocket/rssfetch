use chrono::Local;
use log::{info, warn};
use serde::Deserialize;
use serde_saphyr;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
#[derive(Debug, Deserialize)]
pub struct RSSconfig {
    #[serde(default = "localhost")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "String::new")]
    pub access_key: String,
    #[serde(default = "default_save_path")]
    pub save_path: String,
    #[serde(default = "Vec::new")]
    pub routes: Vec<String>,
    #[serde(default = "String::new")]
    pub api_base: String,
    #[serde(default = "String::new")]
    pub api_key: String,
    #[serde(default = "String::new")]
    pub model_name: String,
    #[serde(default = "default_path")]
    pub file_path: PathBuf,
    #[serde(default = "default_path")]
    pub summary_path: PathBuf,
}

fn localhost() -> String {
    "localhost".into()
}
fn default_port() -> u16 {
    80
}
fn default_save_path() -> String {
    "output".into()
}
fn default_path() -> PathBuf {
    PathBuf::from("output")
}
impl RSSconfig {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open("config.yml")
            .map_err(|e| format!("打开配置文件 config.yml 失败: {e}"))?;

        // 错误分支 6: YAML 语法错误，或字段类型不匹配
        let mut config: Self = 
            serde_saphyr::from_reader(file).map_err(|e| format!("解析 config.yml 失败: {e}"))?;

        if config.routes.is_empty() {
            warn!("config.yml 中没有配置任何 routes，程序无事可做");
            return Ok(config);
        }
        info!(
            "共 {} 个订阅源, host={}:{}",
            config.routes.len(),
            config.host,
            config.port
        );
        // 错误分支 7: 创建输出目录失败（路径是文件、无权限等）
        if config.save_path.is_empty() {
            warn!("save_path 为空, 将直接写入当前目录");
        } else if let Err(e) = fs::create_dir_all(&config.save_path) {
            // 不在此处重复 error!，由 main 的"致命错误"统一输出，避免同一错误打印两遍
            return Err(format!("创建输出目录 [{}] 失败: {e}", config.save_path).into());
        }

        let now = Local::now();
        config.file_path = Path::new(&config.save_path).join(format!("{}.md", now.format("%Y-%m-%d")));
        config.summary_path = Path::new(&config.save_path).join(format!("{}-summary.md", now.format("%Y-%m-%d")));
        //清空文件
        File::create(&config.file_path)?;
        File::create(&config.summary_path)?;
        Ok(config)
    }
}
