"""rssfetch —— RSS 抓取 + LLM 摘要。

分层：
- Rust（PyO3 扩展 ``rssfetch._rssfetch``）：网络请求、RSS 解析、HTML 转文本、调 LLM。
- Python（本包）：配置解析、并发编排、markdown 输出、日志与命令行入口。

命令行::

    python -m rssfetch -c config.yml
"""

from __future__ import annotations

from ._rssfetch import FeedItem, __version__, fetch_feed, html_to_text, summarize
from .config import Config, ConfigError, load_config
from .runner import RunReport, SourceResult, run

__all__ = [
    "Config",
    "ConfigError",
    "FeedItem",
    "RunReport",
    "SourceResult",
    "__version__",
    "fetch_feed",
    "html_to_text",
    "load_config",
    "run",
    "summarize",
]
