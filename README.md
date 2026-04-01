# netfilum

一个课程作业版的 RPC 网络文件系统。

当前结构已经拆成双端：

- `netfilum`：Windows 客户端，负责通过 WinFsp 挂载盘符
- `netfilumd`：Linux / WSL 服务端，负责导出一个目录并响应 RPC 请求

`netfilum up` 仍然保留，但它只是同机 Windows + WSL 场景下的快捷入口。它会通过 `wsl.exe` 启动与 `netfilum.exe` 同目录的 `netfilumd`，然后再执行挂载。

## 前提

- Windows 侧已安装 WinFsp
- Windows 和 Linux / WSL 两侧都已安装 Rust 工具链
- 客户端能够连到服务端的地址和端口

默认地址是 `127.0.0.1:4040`，默认卷标是 `netfilum`。

## 构建

Windows 客户端：

```bash
cargo build --release --bin netfilum
```

Linux / WSL 服务端：

```bash
cargo build --release --bin netfilumd
```

开发阶段也可以直接用 `cargo run`。

## 使用

先在 Linux / WSL 上启动服务端：

```bash
netfilumd --root /home/$USER/netfilum-root --addr 127.0.0.1:4040 --volume-label netfilum
```

再在 Windows 上挂载盘符：

```bash
netfilum mount --addr 127.0.0.1:4040 --mount N: --volume-label netfilum
```

如果服务端在另一台 Linux 机器上，把 `127.0.0.1:4040` 换成可达地址即可。

## 本机 WSL 快捷启动

如果当前就是在 Windows + WSL 的同一台机器上，可以直接用：

```bash
netfilum up --distro Ubuntu --root /home/$USER/netfilum-root --mount N: --addr 127.0.0.1:4040
```

这条命令会：

1. 在指定的 WSL 发行版里启动与 `netfilum.exe` 同目录的 `netfilumd`
2. 等待 RPC 服务就绪
3. 在 Windows 上挂载盘符
4. 当前进程结束时卸载盘符并停止刚刚拉起的服务端

使用 `up` 时，需要把 Windows 客户端 `netfilum.exe` 和 Linux / WSL 服务端 `netfilumd` 放在同一个目录里，并且该目录能被 WSL 访问到。

## 说明

当前实现是自定义 RPC 文件系统，不兼容标准 NFS 协议。

当前范围以课程作业演示为主，默认假设：

- 单用户
- 单客户端挂载
- 无认证
- 无文件锁
- 无 symlink / hardlink / mmap
