# ReadWise (Makepad 重写版)

用 [Makepad 2.0](https://zhanghandong.github.io/makepad-2.0-book/)（Splash DSL + 纯 Rust）重写的 ReadWise 阅读器。
原 Tauri + React 版本保留在 `../readwise`；当前项目的阅读、词汇、AI、统计和本地数据主链路均已迁移到 Rust + Makepad。

- 里程碑规划：[docs/MILESTONES.md](docs/MILESTONES.md)
- 开发日志：[docs/DEVLOG.md](docs/DEVLOG.md)

## 工程结构

```
readwise-makepad/
├── app/          # Makepad UI 应用与平台资源
├── core/         # SQLite、EPUB、词汇、笔记、AI 等纯 Rust 业务逻辑
├── scripts/      # 可复现的数据生成与发布脚本
└── docs/         # 里程碑、部署指南与开发日志
```

## 运行（桌面）

```bash
cargo run -p readwise-app            # 开发构建
cargo run -p readwise-app -- --hot   # 热重载：改 script_mod! 内的 Splash 代码即时生效
cargo run -p readwise-app --release  # 发布构建
```

依赖 makepad dev 分支（git 依赖），首次构建会拉取并编译 makepad，耗时较长属正常。

## macOS 应用包

项目提供可复现的发布脚本：它通过 `cargo-makepad` 构建 release `.app`，写入 ReadWise 产品名、`org.ohmycloud.readwise` Bundle ID、Cargo 版本和应用分类，完成签名校验并生成 ZIP。

```bash
scripts/package_macos.sh
```

产物：

```text
target/dist/ReadWise.app
target/dist/ReadWise-macOS-<version>.zip
```

默认使用 ad-hoc 签名，只适合本机测试。对外分发需使用 Apple Developer 的 Developer ID Application 证书：

```bash
READWISE_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
  scripts/package_macos.sh
```

脚本会启用 hardened runtime 和可信时间戳。公开下载前仍需用 `xcrun notarytool` 提交公证并用 `xcrun stapler` 装订；证书和公证凭据不存入仓库。

## 运行（iOS）

前置条件：完整版 Xcode（App Store 安装，打开一次接受协议，并在 Settings → Platforms 装好 iOS 支持）。

```bash
# 一次性安装（工具 + rustup iOS target，iOS 用 stable 即可）
cargo install --git https://github.com/makepad/makepad --branch dev cargo-makepad
cargo makepad apple ios install-toolchain
```

注意：`--org`/`--app` 等选项必须写在 `run-sim`/`run-device` **之前**（解析器遇到第一个非选项参数即停止收集）。Bundle ID 为 `{org}.{app}`。

```bash
# 模拟器（无需签名；若报模拟器错误，先打开 Simulator.app）
cd readwise-makepad
cargo makepad apple ios --org=org.ohmycloud --app=readwise run-sim -p readwise-app --release
```

真机需要签名三件套（provisioning profile / 证书 / 设备 ID）：

1. iPhone 连 Mac，手机上信任电脑，并开启开发者模式（设置 → 隐私与安全性 → 开发者模式，需重启）。
2. 用 Xcode 新建一个一次性空 App 工程，Bundle Identifier 填 `org.ohmycloud.readwise`，Team 选自己的 Apple ID（免费个人团队即可，签名 7 天有效），真机上 Run 一次——这一步让 Xcode 自动生成证书和 provisioning profile。
3. 列出机器上的签名资源，取三个值（注意 `list` 直接跟在 `apple` 后，不带 `ios`）：

```bash
cargo makepad apple list
```

（跑缺参的 `run-device` 也会自动打印这份清单。）

4. 带全参数跑真机：

```bash
cargo makepad apple ios \
  --org=org.ohmycloud --app=readwise \
  --profile=<profile 名或路径> --cert=<证书标识> --device=<设备标识> \
  run-device -p readwise-app --release
```

## 当前验证入口

功能完成度与待验收项以 [docs/MILESTONES.md](docs/MILESTONES.md) 为准；iOS 真机签名、安装和常见故障见 [docs/ios-deploy-guide.md](docs/ios-deploy-guide.md)。应用数据默认保存在系统 Application Support 下的 `ReadWise` 目录。
