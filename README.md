# uv Python Wrapper

通过 `uv python find --system` 查找系统 Python，并转发命令行参数。

编译：

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
