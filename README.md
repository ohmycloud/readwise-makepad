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

脚本会启用 hardened runtime 和可信时间戳。公开下载前，先把 App Store Connect 专用密码安全存入 macOS 钥匙串（只需一次）：

```bash
xcrun notarytool store-credentials "readwise-notary" \
  --apple-id "your-apple-id@example.com" \
  --team-id "TEAMID" \
  --password "APP-SPECIFIC-PASSWORD"
```

再同时提供 Developer ID 证书和钥匙串 profile：

```bash
READWISE_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
READWISE_NOTARY_PROFILE="readwise-notary" \
  scripts/package_macos.sh
```

脚本会提交 ZIP 并等待公证结果，随后把票据装订到 `.app`，执行 `stapler validate`、`codesign` 与 Gatekeeper `spctl` 校验，并重新生成包含装订票据的最终 ZIP。证书和公证凭据都不存入仓库。

## Linux 应用包

在 Linux x86_64 或 aarch64 主机上运行：

```bash
scripts/package_linux.sh
```

脚本使用本机 ABI 构建 release 可执行文件，生成包含程序、512px 图标和用户级安装器的版本化归档：

```text
target/dist/ReadWise-linux-<架构>-<版本>.tar.gz
```

解压后可直接运行 `./ReadWise`，也可执行 `./install-user.sh` 安装到当前用户的 XDG 目录，无需 root 权限。安装器默认写入 `~/.local/bin` 与 `~/.local/share`；可以用 `XDG_BIN_HOME`、`XDG_DATA_HOME` 覆盖。

## Windows 应用包

在 Windows PowerShell 中运行：

```powershell
pwsh -File scripts/package_windows.ps1
```

脚本通过本机 Windows 工具链构建带应用图标的 `ReadWise.exe`，并输出：

```text
target\dist\ReadWise-windows-<架构>-<版本>.zip
```

默认产物未签名，只适合内部验证。公开分发时可从证书存储按 thumbprint 选择代码签名证书；时间戳服务地址由发布者显式提供，凭据和服务配置不进入仓库：

```powershell
$env:READWISE_WINDOWS_CERT_THUMBPRINT = "<certificate thumbprint>"
$env:READWISE_WINDOWS_TIMESTAMP_URL = "<RFC 3161 timestamp URL>"
pwsh -File scripts/package_windows.ps1
```

设置证书后，脚本要求 Windows SDK 的 `signtool.exe`，并在归档前使用系统 Authenticode API 验证签名。

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

## Android 应用包

先确保 `cargo-makepad` 与项目锁定的 Makepad 版本一致，再安装 arm64 Android 工具链。ReadWise 的 TLS 依赖会编译原生代码，因此必须安装完整 NDK；默认精简 NDK 不够：

```bash
cargo install --git https://github.com/makepad/makepad --branch dev cargo-makepad
cargo makepad android --abi=aarch64 install-toolchain --full-ndk
cargo fetch --locked --target aarch64-linux-android
```

生成用于真机测试、由 Makepad 开发证书签名的 release APK：

```bash
scripts/package_android.sh
```

产物为：

```text
target/dist/ReadWise-android-aarch64-<版本>.apk
```

应用使用 `org.ohmycloud.readwise` 包名、Cargo 版本、API 26 最低版本和 API 35 目标版本。Android 清单只申请 AI/API 通信必需的网络与网络状态权限，不申请相机、麦克风、定位、蓝牙、存储或查询全部应用权限。“导入 EPUB”使用 Android 系统文档选择器的单次授权，选中的文件会先复制到应用私有缓存，无需全局存储权限。

如需额外生成 Play Store AAB，提供发布 keystore 和密码。密码通过环境变量传递，不写入命令行或仓库；key alias 默认从 `cargo makepad android keystore-create` 生成的 `.makepad` sidecar 读取：

```bash
READWISE_ANDROID_KEYSTORE="/secure/path/readwise-upload.keystore" \
MAKEPAD_KEYSTORE_PASS="<password>" \
READWISE_ANDROID_VERSION_CODE="<递增整数>" \
  scripts/package_android.sh
```

没有发布 keystore 时脚本不会生成可能被误认成正式发布包的 AAB。可用 `READWISE_ANDROID_ABI` 覆盖默认 `aarch64`，用 `READWISE_ANDROID_SDK_PATH` 指向自定义 Makepad Android 工具链目录。

## 当前验证入口

功能完成度与待验收项以 [docs/MILESTONES.md](docs/MILESTONES.md) 为准；iOS 真机签名、安装和常见故障见 [docs/ios-deploy-guide.md](docs/ios-deploy-guide.md)。应用数据默认保存在系统 Application Support 下的 `ReadWise` 目录。
