# uv Python Wrapper

通过 `uv python find --system` 查找系统 Python，并转发命令行参数。

## 下载

[下载最新发行版](https://github.com/licyk/uv_python_warpper/releases/latest)，按操作系统和 CPU 架构选择压缩包：

| 平台 | x86_64 / Intel | ARM64 / Apple Silicon |
| --- | --- | --- |
| Windows | [下载 ZIP](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-x86_64-pc-windows-msvc.zip) | [下载 ZIP](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-aarch64-pc-windows-msvc.zip) |
| Linux（glibc） | [下载 tar.gz](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-x86_64-unknown-linux-gnu.tar.gz) | [下载 tar.gz](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-aarch64-unknown-linux-gnu.tar.gz) |
| Linux（musl） | [下载 tar.gz](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-x86_64-unknown-linux-musl.tar.gz) | [下载 tar.gz](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-aarch64-unknown-linux-musl.tar.gz) |
| macOS | [下载 tar.gz](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-x86_64-apple-darwin.tar.gz) | [下载 tar.gz](https://github.com/licyk/uv_python_warpper/releases/latest/download/uv_python_warpper-aarch64-apple-darwin.tar.gz) |

每个压缩包包含 `python` 和 `pip`（Windows 为 `.exe`）、README 和记录版本及源码提交的 `BUILD.json`；发行页面同时提供 `SHA256SUMS`。Linux glibc 包在 Ubuntu 22.04 上构建；musl 包静态链接 musl，可用于 Alpine 等环境。解压后将程序所在目录加入 `PATH`，并先安装 [uv](https://docs.astral.sh/uv/getting-started/installation/)。发行包不包含 uv 或 Python。

## 编译

```powershell
cargo build --release
```

编译产物位于 `target\release`：

```powershell
.\target\release\python.exe --version
.\target\release\python.exe script.py

.\target\release\pip.exe install requests
.\target\release\pip.exe list
```

其中 `pip.exe` 等价于：

```powershell
<实际 Python 路径> -m pip <参数>
```

运行前需要安装并确保 `uv` 已在 `PATH` 中。

调用 `uv` 时会从其 `PATH` 中移除 wrapper 所在目录及指向 wrapper 的 `python` 符号链接所在目录，避免递归调用。路径处理兼容 Windows、Linux 和 macOS；实际 Python 执行时仍保留原始 `PATH`。

Python 直接继承标准输入、标准输出和标准错误，支持管道、文件重定向和终端交互，不额外缓存其输出。输出缓冲行为由 Python 自身决定，需要时可使用 `python -u`。

Linux / macOS 使用 `exec` 替换 wrapper，由 Python 直接处理 Ctrl+C 等信号。Windows 中 wrapper 接住自身的 Ctrl+C / Ctrl+Break 事件，等待 Python 自行处理同一控制台事件并退出，再返回 Python 的退出码。

在 `uv` 查找阶段收到中断时，会取消查找并清理探测子进程；Unix 返回 `128 + 信号编号`，Windows 返回 `0xC000013A`。`uv` 不读取用户的标准输入，查找结果不会混入 Python 的标准输出。

## CI 与发行

`ci.yml` 是唯一自动入口：PR 和 `main` 推送先调用 `checks.yml`，在六种系统与架构组合上运行 Clippy 和测试，再由 `build.yml` 构建八个发行包并验证真实 uv / Python / pip 调用。所有构建成功后汇总校验和；只有需要发行时才调用 `release.yml`，将同一次运行的产物发布到 GitHub Release。发行工作流不重复测试或构建，也不发布到 crates.io、PyPI 或其他平台。

推送到 `main` 时，如果 `Cargo.toml` 的 `[package].version` 相比本次推送前发生变化，就自动发布 `v<版本>`；请同步更新 `Cargo.lock` 中本包的版本。只修改依赖或其他清单字段不会发行。首次推送（没有比较基线）仅检查和构建；首次发行可在 Actions 的 CI 中选择 `main`，手动运行 `release` 模式。

手动 CI 还提供 `check`（仅检查）和 `build`（检查并构建）模式。标签由发行步骤生成，标签推送不会触发另一轮 CI。既有标签必须指向同一源码提交；重跑会验证已发行的附件，不覆盖正式发行。预发行版本不会替换最新正式版。发布使用仓库内置的 `GITHUB_TOKEN`，只有发行 job 具有 `contents: write` 权限，不需要额外发布令牌。

CI 辅助脚本仅使用 Python 3.11+ 标准库，本地可运行：

```bash
cargo test --locked
python scripts/ci.py plan
```

测试与构建使用的 Rust 版本及平台矩阵统一定义在 `scripts/ci.py`。Actions 构建产物保留 7 天，GitHub Release 附件长期提供下载。
