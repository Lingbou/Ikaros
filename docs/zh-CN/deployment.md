# 部署

Ikaros 的部署能力仍处于 pre-MVP 阶段。当前仓库里支持的部署 artifact 是本地
Docker 镜像，用于开发和 smoke test。它不是托管、多用户或已经生产加固的部署方式。

## Docker 镜像

在仓库根目录构建：

```bash
docker build -f docker/Dockerfile -t ikaros:local .
```

Dockerfile 使用 Rust builder stage 构建，并只把最终 `ikaros` 二进制复制到 Debian
slim runtime 镜像。Runtime 镜像以非特权 `ikaros` 用户运行，设置
`IKAROS_HOME=/data/ikaros`，并把 `/workspace` 作为工作目录。

运行单个命令：

```bash
docker run --rm -it \
  -e IKAROS_HOME=/data/ikaros \
  -v ikaros-home:/data/ikaros \
  -v "$PWD":/workspace \
  -w /workspace \
  ikaros:local config show
```

在仓库根目录使用 Compose：

```bash
docker compose -f docker/compose.yml run --rm ikaros --help
docker compose -f docker/compose.yml run --rm ikaros setup \
  --api-key "$MODEL_API_KEY" \
  --base-url https://api.example.com/v1 \
  --model provider-model-id
docker compose -f docker/compose.yml run --rm ikaros
```

## 状态和密钥

不要把 provider credential bake 进镜像。Runtime 配置和本地状态应放在
`/data/ikaros`，并通过 Docker volume 或 bind mount 持久化。镜像期望明文 provider
credential 位于 `/data/ikaros/config.yaml`，这和普通本地 `IKAROS_HOME/config.yaml`
契约一致。

`/workspace` 的仓库挂载用于本地开发工作流。它仍然使用 host 运行时相同的 harness 和
workspace policy 假设：这里的 Docker 是 CLI 打包方式，不是完整进程沙箱的替代品。

## 安装脚本和打包入口

仓库现在也包含早期打包入口：

- `install.sh` 和 `install.ps1`：用于安装本地 release binary。
- `flake.nix`：用于 Nix 开发和包构建实验。
- `packaging/arch/PKGBUILD`：用于 Arch 打包实验。
- cargo-dist metadata：为后续生成 release artifact 做准备。

这些文件目前是维护者和开发者使用的 packaging scaffold。它们可能依赖 tag release
artifact、本地构建或平台工具，还不代表已经有稳定发布渠道。

## 限制

- 还没有定义已发布镜像。
- 已经有 packaging template，但还不保证 Nix cache、Windows package feed、Homebrew
  tap、AUR package 或 cargo-dist release artifact。
- 部署镜像和 `execution.sandbox.backend: docker` 进程沙箱是两件事。容器化命令/测试执行
  使用 `execution.sandbox.image` 指定镜像。
