# netfilum

一个用于课程作业演示的 RPC 网络文件系统。

程序分为两端：

- `netfilum`：Windows 客户端，负责通过 WinFsp 挂载盘符
- `netfilumd`：Linux / WSL 服务端，负责导出一个目录并响应 RPC 请求

同机 Windows + WSL 场景可使用 `netfilum up` 快捷启动。它会通过 `wsl.exe` 启动与 `netfilum.exe` 同目录的 `netfilumd`，然后执行挂载。

## 架构

```mermaid
flowchart LR
    A["Windows 应用<br/>资源管理器 / 编辑器"] --> B["netfilum<br/>WinFsp 挂载客户端"]
    B --> C["自定义 RPC<br/>可选密码认证 + 加密传输"]
    C --> D["netfilumd<br/>Linux / WSL 服务端"]
    D --> E["导出目录<br/>std::fs"]
```

核心分工可以简单理解成这样：

- `netfilum` 和 `netfilumd` 之间的 RPC 协议、请求分发、路径约束、文件语义映射，以及按密码启用的认证和加密传输，是这个项目自己实现的
- WinFsp 负责把 Windows 用户态文件系统接到盘符上，让 `netfilum` 能作为一个可挂载盘工作
- `serde` 和 `postcard` 负责消息编解码，`clap` 负责命令行解析，`filetime` 负责时间戳设置
- 服务端真正落到磁盘上的文件操作，底层使用的是 Rust 标准库 `std::fs`

## 运行前提

- Windows 侧已安装 WinFsp
- 客户端能够连到服务端的地址和端口
- 使用 `netfilum up` 时，`netfilum.exe` 和 `netfilumd` 需要位于同一目录，并且该目录能被 WSL 访问

默认地址是 `127.0.0.1:4040`，默认卷标是 `netfilum`。
默认密码是空字符串。此时会使用明文传输，客户端和服务端都会输出警告。设置非空密码后会启用密码认证和加密传输。

## 从源码构建

如果需要从源码构建：

- Windows 侧需要 Rust 工具链，用于构建 `netfilum`
- Linux / WSL 侧需要 Rust 工具链，用于构建 `netfilumd`

Windows 客户端：

```bash
cargo build --release --bin netfilum
```

Linux / WSL 服务端：

```bash
cargo build --release --bin netfilumd
```

从源码调试时也可以直接使用 `cargo run`。

## 使用

先在 Linux / WSL 上启动服务端：

```bash
netfilumd --root /home/$USER/netfilum-root --addr 127.0.0.1:4040 --volume-label netfilum --password demo-pass
```

再在 Windows 上挂载盘符：

```bash
netfilum mount --addr 127.0.0.1:4040 --mount N: --volume-label netfilum --password demo-pass
```

如果服务端在另一台 Linux 机器上，把 `127.0.0.1:4040` 换成可达地址即可。

## 本机 WSL 快捷启动

同机 Windows + WSL 场景可以直接用：

```bash
netfilum up --distro Ubuntu --root /home/$USER/netfilum-root --mount N: --addr 127.0.0.1:4040 --password demo-pass
```

这条命令会：

1. 在指定的 WSL 发行版里启动与 `netfilum.exe` 同目录的 `netfilumd`
2. 等待 RPC 服务就绪
3. 在 Windows 上挂载盘符
4. 当前进程结束时卸载盘符并停止刚刚拉起的服务端

## 运行行为

- `netfilum mount` 和 `netfilum up` 都以前台进程形式保持挂载，按 `Ctrl+C` 会卸载盘符
- 如果 RPC 服务端断开，客户端会检测到连接中断并自动执行卸载
- Windows 侧显示的卷总容量和可用容量来自服务端导出目录所在文件系统

## 说明

本项目实现的是自定义 RPC 文件系统，不兼容标准 NFS 协议。

本项目以课程作业演示为目标，默认假设：

- 单用户
- 单客户端挂载
- 共享密码模型
- 无文件锁
- 无 symlink / hardlink / mmap
