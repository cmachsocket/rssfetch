# rssfetch

抓取 RSS 订阅源 → 输出当日 markdown → 调用 OpenAI 兼容接口生成摘要。

[maturin](https://github.com/PyO3/maturin) 混合项目：**Rust(PyO3)** 负责网络请求、
RSS 解析、HTML 转纯文本、调用 LLM；**Python** 负责配置解析、并发编排、文件输出与日志。

```
rssfetch/
├── Cargo.toml            # 扩展模块 crate（lib: _rssfetch, cdylib + rlib）
├── pyproject.toml        # maturin 构建后端（abi3，CPython 3.9+）
├── .github/workflows/    # CI（lint + build）与 Release（多平台 wheel -> PyPI）
├── src/
│   ├── lib.rs            # PyO3 绑定层：fetch_feed / html_to_text / summarize / FeedItem
│   ├── fetch.rs          # HTTP 拉取 + RSS 解析
│   └── summarize.rs      # chat completion 摘要
└── python/rssfetch/      # 纯 Python 包
    ├── __init__.py       # 对外 API
    ├── __main__.py       # python -m rssfetch
    ├── cli.py            # 命令行参数与退出码
    ├── config.py         # config.yml → dataclass
    └── runner.py         # 并发抓取 → 过滤 → markdown → 摘要
```

## 构建安装

```bash
# 开发模式（就地编译并安装进当前虚拟环境）
maturin develop

# 或构建 wheel（abi3，一个 wheel 覆盖 CPython 3.9+）
maturin build --release
pip install target/wheels/rssfetch-*.whl
```

## 发布到 PyPI

版本号在 `Cargo.toml` 和 `pyproject.toml` 中各写一处，两者需保持一致。

```bash
# 1. 改版本号（例：0.1.0 -> 0.2.0），提交
# 2. 打 tag 触发发布
git tag v0.2.0 && git push origin main --tags
```

[`.github/workflows/release.yml`](.github/workflows/release.yml) 会构建 6 个平台的
abi3 wheel（Linux x86_64/aarch64、macOS x86_64/aarch64、Windows x86_64）加 sdist，
经 PyPI 的 trusted publishing（OIDC）自动上传，无需在仓库里存 API token。

首次发布前需要在 PyPI 上为 `cmachsocket/rssfetch` 配置 trusted publisher
（Settings → Publishing → GitHub，填 `cmachsocket/rssfetch` + workflow 名 `release.yml`）。

不传 tag 也可以在 Actions 页面手动跑该 workflow（`dry_run: true` 时只构建不上传）。

本地验证打包结果：

```bash
maturin sdist --out dist && twine check dist/*
```

## 使用

```bash
python -m rssfetch                      # 使用 ./config.yml
python -m rssfetch -c prod.yml -l debug
python -m rssfetch --days 3 --no-summary
python -m rssfetch -n                   # dry-run，只打印 URL
```

输出：`output/YYYY-MM-DD.md`（正文）与 `output/YYYY-MM-DD-summary.md`（摘要）。

## 配置 `config.yml`

```yaml
host: rss.cmach.top
port: 80
access_key: your-key
save_path: output
routes:
  - /ymgal/article
  - /ithome/ranking/24h

api_base: https://api.example.com/v1
api_key: sk-xxxx
model_name: your-model
```

除 `routes` 外都有默认值；未配置 `api_*` / `model_name` 时会跳过摘要生成并给出告警。

## 作为库使用

```python
from rssfetch import Config, fetch_feed, html_to_text, load_config, run, summarize

items = fetch_feed("https://example.com/feed.xml", timeout=20.0)
print(items[0].title, items[0].published)

config = load_config("config.yml")
report = await run(config)          # 协程：内部用 asyncio.to_thread 并发抓取
```

Rust 侧导出：`FeedItem`（`title` / `link` / `pub_date` / `published` / `description` /
`content` / `to_dict()`）、`fetch_feed(url, timeout=30.0)`、
`html_to_text(html, width=None)`、`summarize(text, *, api_base, api_key, model,
system_prompt=None, timeout=180.0)`。
