//! rssfetch 的 PyO3 绑定层。
//!
//! 职责划分：**Rust 只做重活**（网络请求、RSS 解析、HTML 转纯文本、调用 LLM），
//! 配置解析、并发编排、文件输出与日志全部交给 Python 侧的 `rssfetch` 包，
//! 这样既保留 Rust 的性能，又把流程控制留在 Python 里方便扩展。

mod fetch;
mod llm;

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::runtime::Runtime;

/// 全进程共用一个多线程 tokio runtime。
///
/// PyO3 的接口是同步的（Python 侧用 `asyncio.to_thread` 拿到并发），
/// 内部用 `block_on` 驱动 async 代码，等待期间通过 `Python::detach` 释放 GIL。
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("创建 tokio runtime 失败")
    })
}

/// Python 传进来的秒数 → `Duration`，非法值（负数 / NaN / 0）回退到默认值。
fn seconds(value: f64, default: f64) -> Duration {
    if value.is_finite() && value > 0.0 {
        Duration::from_secs_f64(value)
    } else {
        Duration::from_secs_f64(default)
    }
}

/// 单条 RSS 条目（对应 `rss::Item` 的常用字段）。
// `skip_from_py_object`: 条目只会由 Rust 创建后交给 Python，不需要从 Python 反向提取。
#[pyclass(frozen, skip_from_py_object, module = "rssfetch._rssfetch")]
#[derive(Debug, Clone, Default)]
pub struct FeedItem {
    pub title: String,
    pub link: Option<String>,
    /// 原始的 RFC 2822 时间字符串，可能缺失。
    pub pub_date: Option<String>,
    /// `pub_date` 解析出的 Unix 时间戳（秒）。缺失或格式非法时为 `None`，
    /// 由 Python 侧决定兜底策略并告警——不在 Rust 里静默按 epoch 处理。
    pub published: Option<f64>,
    pub description: Option<String>,
    pub content: Option<String>,
}

#[pymethods]
impl FeedItem {
    #[new]
    #[pyo3(signature = (title, *, link=None, pub_date=None, description=None, content=None))]
    fn new(
        title: String,
        link: Option<String>,
        pub_date: Option<String>,
        description: Option<String>,
        content: Option<String>,
    ) -> Self {
        Self {
            title,
            link,
            published: None,
            pub_date,
            description,
            content,
        }
    }

    #[getter]
    fn title(&self) -> &str {
        &self.title
    }

    #[getter]
    fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }

    #[getter]
    fn pub_date(&self) -> Option<&str> {
        self.pub_date.as_deref()
    }

    #[getter]
    fn published(&self) -> Option<f64> {
        self.published
    }

    #[getter]
    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    #[getter]
    fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// 转成普通 dict，方便序列化或交给 json/yaml 处理。
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("title", &self.title)?;
        dict.set_item("link", self.link.as_deref())?;
        dict.set_item("pub_date", self.pub_date.as_deref())?;
        dict.set_item("published", self.published)?;
        dict.set_item("description", self.description.as_deref())?;
        dict.set_item("content", self.content.as_deref())?;
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "FeedItem(title={:?}, pub_date={:?}, link={:?})",
            self.title, self.pub_date, self.link
        )
    }
}

/// 抓取并解析一个 RSS 源，返回条目列表。
///
/// 阻塞式调用：内部释放 GIL，所以 Python 侧用 `asyncio.to_thread` 包一层即可并发。
#[pyfunction]
#[pyo3(signature = (url, timeout=30.0))]
fn fetch_feed(py: Python<'_>, url: &str, timeout: f64) -> PyResult<Vec<FeedItem>> {
    let timeout = seconds(timeout, 30.0);
    py.detach(|| runtime().block_on(fetch::feed(url, timeout)))
        .map_err(PyIOError::new_err)
}

/// HTML → 纯文本。`width` 为折行宽度，缺省时按原文长度的 3 倍估算。
#[pyfunction]
#[pyo3(signature = (html, width=None))]
fn html_to_text(html: &str, width: Option<usize>) -> PyResult<String> {
    let width = width.unwrap_or_else(|| html.len().saturating_mul(3));
    fetch::html_to_text(html, width).map_err(PyValueError::new_err)
}

/// 调用 OpenAI 兼容接口生成摘要。
#[pyfunction]
#[pyo3(signature = (text, *, api_base, api_key, model, system_prompt=None, timeout=180.0))]
fn summarize(
    py: Python<'_>,
    text: &str,
    api_base: &str,
    api_key: &str,
    model: &str,
    system_prompt: Option<&str>,
    timeout: f64,
) -> PyResult<String> {
    let system_prompt = system_prompt.unwrap_or(llm::DEFAULT_SYS_PROMPT);
    let timeout = seconds(timeout, 180.0);
    py.detach(|| {
        runtime().block_on(llm::summarize(
            text,
            api_base,
            api_key,
            model,
            system_prompt,
            timeout,
        ))
    })
    .map_err(PyRuntimeError::new_err)
}

#[pymodule]
fn _rssfetch(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<FeedItem>()?;
    m.add_function(wrap_pyfunction!(fetch_feed, m)?)?;
    m.add_function(wrap_pyfunction!(html_to_text, m)?)?;
    m.add_function(wrap_pyfunction!(summarize, m)?)?;
    Ok(())
}
