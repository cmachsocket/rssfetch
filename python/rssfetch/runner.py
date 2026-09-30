"""编排：并发抓取 → 时间窗口过滤 → 生成 markdown → 调 LLM 摘要。

并发策略：Rust 侧的 ``fetch_feed`` 是会释放 GIL 的阻塞函数，所以直接丢进
``asyncio.to_thread``，由 Python 的线程池负责并行，无需在 Rust 里再搭一套 runtime。
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Iterable, List, Optional, Sequence

from ._rssfetch import FeedItem, fetch_feed, html_to_text, summarize
from .config import Config

log = logging.getLogger("rssfetch")

#: 只保留最近一天的条目（与原 Rust 版行为一致）
DEFAULT_MAX_AGE = timedelta(days=1)
DEFAULT_FETCH_TIMEOUT = 30.0
DEFAULT_LLM_TIMEOUT = 180.0
NO_TITLE = "(无标题)"
NO_PUB_DATE = "(无发布时间)"


@dataclass
class SourceResult:
    """单个订阅源的处理结果。失败只记录原因，不中断其他源。"""

    route: str
    url: str
    items: List[FeedItem] = field(default_factory=list)
    error: Optional[str] = None

    @property
    def ok(self) -> bool:
        return self.error is None


@dataclass
class RunReport:
    """一次完整运行的汇总。"""

    sources: List[SourceResult] = field(default_factory=list)
    file_path: Optional[Path] = None
    summary_path: Optional[Path] = None
    item_count: int = 0

    @property
    def failed(self) -> List[SourceResult]:
        return [source for source in self.sources if not source.ok]

    @property
    def ok(self) -> bool:
        return not self.failed


async def _fetch_source(route: str, url: str, timeout: float) -> SourceResult:
    try:
        # Rust 侧是同步阻塞调用，放进线程池才能真正并发。
        items = await asyncio.to_thread(fetch_feed, url, timeout)
    except Exception as exc:  # noqa: BLE001 - 订阅源失败不应中断整体流程
        # 错误分支: 单个订阅源抓取/解析失败
        log.debug("订阅源失败 [%s]", route, exc_info=True)
        return SourceResult(route=route, url=url, error=str(exc))
    return SourceResult(route=route, url=url, items=list(items))


def _to_local(published: float) -> datetime:
    return datetime.fromtimestamp(published, tz=timezone.utc).astimezone()


def fresh_items(
    items: Iterable[FeedItem],
    max_age: timedelta = DEFAULT_MAX_AGE,
    now: Optional[datetime] = None,
) -> List[FeedItem]:
    """过滤掉发布时间无法解析或早于 ``now - max_age`` 的条目。"""
    now = now or datetime.now().astimezone()
    cutoff = now - max_age
    kept: List[FeedItem] = []
    for item in items:
        if item.published is None:
            # 时间缺失/非法时无法判断新鲜度，保守丢弃并留痕。
            # （原 Rust 版会 fallback 到 epoch，结果同样是丢弃，但没日志。）
            log.warning(
                "发布时间无法解析 标题=%r 值=%r（该条目将被丢弃）",
                item.title,
                item.pub_date,
            )
            continue
        if _to_local(item.published) < cutoff:
            log.info(
                "Skipping item %r published on %s (older than %s)",
                item.title,
                item.pub_date,
                max_age,
            )
            continue
        kept.append(item)
    return kept


def describe_item(item: FeedItem) -> Sequence[str]:
    """把一条目渲染成 markdown 行（描述做 HTML→纯文本转换）。"""
    lines = [f"- 标题: {item.title}", f"  发布时间: {item.pub_date or NO_PUB_DATE}"]
    if item.description:
        try:
            text = html_to_text(item.description).strip()
        except ValueError as exc:
            # 转换失败不算致命错误，退回原始 HTML 并留痕。
            log.warning("HTML 转文本失败 标题=%r: %s", item.title, exc)
            text = item.description
        lines.append(f"  描述: {text}")
    return lines


def render_markdown(sources: Sequence[SourceResult]) -> str:
    """拼出当日正文（markdown）。"""
    blocks: List[str] = []
    for source in sources:
        if not source.items:
            continue
        blocks.append(f"## {source.route}")
        for item in source.items:
            blocks.extend(describe_item(item))
            if item.content:
                log.debug("  内容: %s", item.content)
    return "\n".join(blocks) + ("\n" if blocks else "")


async def run(
    config: Config,
    *,
    max_age: timedelta = DEFAULT_MAX_AGE,
    fetch_timeout: float = DEFAULT_FETCH_TIMEOUT,
    llm_timeout: float = DEFAULT_LLM_TIMEOUT,
    do_summary: bool = True,
    dry_run: bool = False,
) -> RunReport:
    """执行一次完整的抓取 + 摘要流程。

    Args:
        config: 已校验的配置。
        max_age: 条目时间窗口，窗口外的条目会被丢弃。
        fetch_timeout: 单个订阅源的 HTTP 超时（秒）。
        llm_timeout: LLM 请求超时（秒）。
        do_summary: 是否生成 LLM 摘要。
        dry_run: 只抓取并打印，不写任何文件。

    Returns:
        RunReport: 各订阅源结果与输出文件路径。
    """
    report = RunReport()

    if not config.routes:
        log.warning("配置中没有任何 routes，程序无事可做")
        return report

    log.info("共 %d 个订阅源, host=%s:%d", len(config.routes), config.host, config.port)

    if dry_run:
        for url in config.feed_urls:
            log.info("[dry-run] %s", url)
        return report

    # 错误分支: 创建输出目录失败（路径是文件、无权限等）
    try:
        config.output_dir.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise RuntimeError(f"创建输出目录 [{config.output_dir}] 失败: {exc}") from exc

    sources = await asyncio.gather(
        *(
            _fetch_source(route, config.feed_url(route), fetch_timeout)
            for route in config.routes
        )
    )
    report.sources = list(sources)

    for source in sources:
        if source.ok:
            source.items = fresh_items(source.items, max_age)
            log.info("订阅源完成 [%s] %d 个条目", source.route, len(source.items))
        else:
            # 逐个汇总失败原因：原 Rust 版 join_all 的返回值被丢弃，失败毫无痕迹。
            log.error("%s", source.error)

    if report.failed:
        log.error("%d/%d 个订阅源处理失败", len(report.failed), len(report.sources))
    else:
        log.info("全部 %d 个订阅源处理成功", len(report.sources))

    markdown = render_markdown(report.sources)
    report.item_count = sum(len(source.items) for source in report.sources)
    if not markdown:
        log.warning("没有可写出的条目，跳过输出")
        return report

    # 错误分支: 打不开输出文件（目录不存在、无写权限等）
    try:
        config.file_path.write_text(markdown, encoding="utf-8")
    except OSError as exc:
        raise RuntimeError(f"写入输出文件 [{config.file_path}] 失败: {exc}") from exc
    report.file_path = config.file_path
    log.info("输出文件: %s（%d 条）", config.file_path, report.item_count)

    if do_summary:
        await _write_summary(config, markdown, llm_timeout, report)

    return report


async def _write_summary(
    config: Config, markdown: str, llm_timeout: float, report: RunReport
) -> None:
    if not config.has_llm:
        log.warning("未配置 api_base / api_key / model_name，跳过摘要生成")
        return

    try:
        # 注意：Rust 侧签名是 summarize(text, *, api_base, api_key, model, ...),
        # 后面几个是 keyword-only，必须按关键字传。
        summary = await asyncio.to_thread(
            summarize,
            markdown,
            api_base=config.api_base,
            api_key=config.api_key,
            model=config.model_name,
            timeout=llm_timeout,
        )
    except Exception as exc:  # noqa: BLE001 - 摘要失败不应让已抓到的内容白跑
        log.error("生成摘要失败: %s", exc)
        return

    try:
        config.summary_path.write_text(summary + "\n", encoding="utf-8")
    except OSError as exc:
        log.error("写入摘要文件 [%s] 失败: %s", config.summary_path, exc)
        return
    report.summary_path = config.summary_path
    log.info("摘要文件: %s", config.summary_path)
