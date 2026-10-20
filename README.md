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

安装 Rust 工具链（包含 Cargo）后，在 Windows、Linux 或 macOS 的项目根目录执行：

```sh
cargo build --release --locked
```

默认构建当前系统和 CPU 架构的程序，产物位于 `target/release/`：

| 平台 | Python wrapper | pip wrapper |
| --- | --- | --- |
| Windows | `python.exe` | `pip.exe` |
| Linux / macOS | `python` | `pip` |

## 使用说明

运行前需要安装并确保 `uv` 已在 `PATH` 中。可以将编译产物或解压后的程序所在目录加入 `PATH`，也可以通过路径直接运行。

Windows（PowerShell）：

```powershell
.\target\release\python.exe --version
.\target\release\python.exe script.py

.\target\release\pip.exe install requests
.\target\release\pip.exe list
```

Linux / macOS：

```sh
./target/release/python --version
./target/release/python script.py

./target/release/pip install requests
./target/release/pip list
```

其中 `pip`（Windows 为 `pip.exe`）等价于：

```text
<实际 Python 路径> -m pip <参数>
```

## 运行说明

调用 `uv` 时会从其 `PATH` 中移除 wrapper 所在目录及指向 wrapper 的 `python` 符号链接所在目录，避免递归调用。路径处理兼容 Windows、Linux 和 macOS；实际 Python 执行时仍保留原始 `PATH`。

Python 直接继承标准输入、标准输出和标准错误，支持管道、文件重定向和终端交互，不额外缓存其输出。输出缓冲行为由 Python 自身决定，需要时可使用 `python -u`。

Linux / macOS 使用 `exec` 替换 wrapper，由 Python 直接处理 Ctrl+C 等信号。Windows 中 wrapper 接住自身的 Ctrl+C / Ctrl+Break 事件，等待 Python 自行处理同一控制台事件并退出，再返回 Python 的退出码。

在 `uv` 查找阶段收到中断时，会取消查找并清理探测子进程；Unix 返回 `128 + 信号编号`，Windows 返回 `0xC000013A`。`uv` 不读取用户的标准输入，查找结果不会混入 Python 的标准输出。
