"""config.yml 的读取与校验。

配置格式与原 Rust 版保持一致（``host`` / ``port`` / ``access_key`` / ``save_path`` /
``routes`` / ``api_base`` / ``api_key`` / ``model_name``），只是解析搬到了 Python 侧。
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field, fields
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional
from urllib.parse import quote

import yaml

log = logging.getLogger("rssfetch")

DEFAULT_CONFIG_PATH = Path("config.yml")


class ConfigError(Exception):
    """配置文件缺失、语法错误或字段类型不合法。"""


@dataclass
class Config:
    """一次运行所需的全部配置。"""

    host: str = "localhost"
    port: int = 80
    access_key: str = ""
    save_path: str = "output"
    routes: List[str] = field(default_factory=list)
    api_base: str = ""
    api_key: str = ""
    model_name: str = ""
    #: 配置文件路径，仅用于日志排查与默认的相对路径基准。
    source: Optional[Path] = None

    @property
    def feed_urls(self) -> List[str]:
        """各订阅源的完整 URL（含 access_key 查询参数）。"""
        return [self.feed_url(route) for route in self.routes]

    def feed_url(self, route: str) -> str:
        url = f"http://{self.host}:{self.port}{route}"
        if not self.access_key:
            return url
        # route 里可能已经带查询参数，用 & 而不是硬拼 ?
        separator = "&" if "?" in route else "?"
        return f"{url}{separator}key={quote(self.access_key, safe='')}"

    @property
    def has_llm(self) -> bool:
        """三项 LLM 配置齐全才允许调用摘要接口。"""
        return bool(self.api_base and self.api_key and self.model_name)

    @property
    def output_dir(self) -> Path:
        return Path(self.save_path) if self.save_path else Path(".")

    def _dated(self, suffix: str, now: Optional[datetime] = None) -> Path:
        now = now or datetime.now()
        return self.output_dir / f"{now:%Y-%m-%d}{suffix}"

    @property
    def file_path(self) -> Path:
        """当日正文输出路径。"""
        return self._dated(".md")

    @property
    def summary_path(self) -> Path:
        """当日摘要输出路径。"""
        return self._dated("-summary.md")


def load_config(path: Path | str = DEFAULT_CONFIG_PATH) -> Config:
    """读取并校验配置文件。

    Args:
        path: config.yml 的路径。

    Raises:
        ConfigError: 文件打不开、YAML 语法错误、顶层不是映射或字段类型不合法。
    """
    path = Path(path)
    try:
        # 错误分支 1: 配置文件不存在 / 无读权限
        with path.open("r", encoding="utf-8") as fp:
            raw = yaml.safe_load(fp)
    except OSError as exc:
        raise ConfigError(f"打开配置文件 {path} 失败: {exc}") from exc
    # 错误分支 2: YAML 语法错误
    except yaml.YAMLError as exc:
        raise ConfigError(f"解析 {path} 失败: {exc}") from exc

    # 错误分支 3: 空文件 / 顶层不是映射
    if raw is None:
        raw = {}
    if not isinstance(raw, dict):
        raise ConfigError(f"{path} 顶层必须是键值映射，实际是 {type(raw).__name__}")

    known = {f.name for f in fields(Config)} - {"source"}
    unknown = set(raw) - known
    if unknown:
        # 不直接报错：多出来的键多半是给人看的注释性配置，忽略即可，但要说一声。
        log.warning(
            "%s 中存在未知配置项，将被忽略: %s", path, ", ".join(sorted(unknown))
        )

    values = {key: raw[key] for key in known if key in raw}
    # 错误分支 4: 字段类型不匹配（如 port 写成字符串、routes 写成字符串）
    try:
        config = Config(**values)
    except TypeError as exc:
        raise ConfigError(f"{path} 字段类型不合法: {exc}") from exc

    config.port = _as_port(config.port, path)
    config.routes = _as_routes(config.routes, path)
    config.source = path
    return config


def _as_port(value: Any, path: Path) -> int:
    try:
        port = int(value)
    except (TypeError, ValueError) as exc:
        raise ConfigError(f"{path} 中 port 不是整数: {value!r}") from exc
    if not 1 <= port <= 65535:
        raise ConfigError(f"{path} 中 port 超出范围 1-65535: {port}")
    return port


def _as_routes(value: Any, path: Path) -> List[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ConfigError(f"{path} 中 routes 必须是字符串列表，实际是 {value!r}")
    return [item.strip() for item in value if item.strip()]


def describe(config: Config) -> Dict[str, Any]:
    """给日志用的配置摘要（不含密钥）。"""
    return {
        "host": f"{config.host}:{config.port}",
        "routes": len(config.routes),
        "save_path": str(config.output_dir),
        "llm": config.model_name or "(未配置)",
    }
