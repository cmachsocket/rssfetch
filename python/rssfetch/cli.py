"""命令行入口：``python -m rssfetch`` / ``rssfetch``。"""

from __future__ import annotations

import argparse
import asyncio
import logging
import sys
from datetime import timedelta
from typing import List, Optional

from . import __version__
from .config import DEFAULT_CONFIG_PATH, ConfigError, describe, load_config
from .runner import (
    DEFAULT_FETCH_TIMEOUT,
    DEFAULT_LLM_TIMEOUT,
    DEFAULT_MAX_AGE,
    run,
)

LOG_FORMAT = "%(asctime)s.%(msecs)03d [%(levelname)s] %(message)s"
LOG_DATE_FORMAT = "%Y-%m-%d %H:%M:%S"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="rssfetch",
        description="抓取 RSS 订阅源，输出当日 markdown 并生成 LLM 摘要",
    )
    parser.add_argument(
        "-c",
        "--config",
        type=str,
        default=str(DEFAULT_CONFIG_PATH),
        help="配置文件路径（默认: %(default)s）",
    )
    parser.add_argument(
        "-l",
        "--log-level",
        default="info",
        choices=["debug", "info", "warning", "error"],
        help="日志级别（默认: %(default)s）",
    )
    parser.add_argument(
        "--days",
        type=float,
        default=DEFAULT_MAX_AGE.total_seconds() / 86400,
        help="只保留最近 N 天的条目（默认: %(default)s）",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_FETCH_TIMEOUT,
        help="单个订阅源的 HTTP 超时秒数（默认: %(default)s）",
    )
    parser.add_argument(
        "--llm-timeout",
        type=float,
        default=DEFAULT_LLM_TIMEOUT,
        help="LLM 请求超时秒数（默认: %(default)s）",
    )
    parser.add_argument(
        "--no-summary", action="store_true", help="只抓取，不调用 LLM 生成摘要"
    )
    parser.add_argument(
        "-n", "--dry-run", action="store_true", help="只打印将要抓取的 URL，不抓不写"
    )
    parser.add_argument(
        "--version", action="version", version=f"rssfetch {__version__}"
    )
    return parser


def main(argv: Optional[List[str]] = None) -> int:
    """CLI 主函数。返回进程退出码，0 表示成功。"""
    args = build_parser().parse_args(argv)

    logging.basicConfig(
        level=getattr(logging, args.log_level.upper()),
        format=LOG_FORMAT,
        datefmt=LOG_DATE_FORMAT,
    )
    log = logging.getLogger("rssfetch")

    try:
        config = load_config(args.config)
    except ConfigError as exc:
        log.error("%s", exc)
        return 2

    log.info("rssfetch 启动: %s", describe(config))
    try:
        report = asyncio.run(
            run(
                config,
                max_age=timedelta(days=args.days),
                fetch_timeout=args.timeout,
                llm_timeout=args.llm_timeout,
                do_summary=not args.no_summary,
                dry_run=args.dry_run,
            )
        )
    except RuntimeError as exc:
        # 致命错误统一经由 log 输出，同时保留非 0 退出码，便于 crontab / CI 感知失败。
        log.error("致命错误: %s", exc)
        return 1
    except KeyboardInterrupt:
        log.error("已被用户中断")
        return 130

    log.info(
        "rssfetch 结束: %d 条, 失败 %d/%d 个订阅源",
        report.item_count,
        len(report.failed),
        len(report.sources),
    )
    return 0 if report.ok else 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
