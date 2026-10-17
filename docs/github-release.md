# GitHub Release 发布

工作流 `.github/workflows/release.yml` 在推送 `v<版本>` 标签时，在各平台原生 runner 构建全部产物；全部成功后创建 **draft Release**，附带 SHA256SUMS.txt。手动运行只生成 Actions artifacts，便于先验证。标签版本必须与 `app/Cargo.toml` 一致。

| 平台 | 产物 | 架构 |
| --- | --- | --- |
| Windows | ZIP，内含 EXE | x64 |
| Linux | tar.gz，内含用户级安装器 | x86_64 |
| macOS | ZIP，内含 APP | arm64 / x86_64 |
| Android | APK | arm64 |
| iOS | 未签名真机 IPA | arm64 |

首次使用：提交发布相关改动，推送到 GitHub，先在 Actions → Release → Run workflow 验证；然后创建并推送与 Cargo 版本一致的标签，例如 `git tag v0.1.0`、`git push origin v0.1.0`。检查草稿附件及安装结果后发布草稿。

构建工具从 Cargo.lock 提取 Makepad 提交，避免安装漂移的 dev 分支。`python3 scripts/install_release_tools.py` 会在 target/release-tools 内安装匹配的工具，并增加 `apple ios build-device` 命令：上游 `build` 只构建模拟器，此命令仅切换到真机 target，保留原有资源、图标和 plist 打包。Python 需要 3.11+。

本地 iOS 打包：安装 Xcode，执行上述工具安装，将 `target/release-tools/installed/bin` 加入 PATH，再执行 `cargo makepad apple ios install-toolchain`、`cargo fetch --locked --target aarch64-apple-ios`、`bash scripts/package_ios.sh`。默认输出 `target/dist/ReadWise-iOS-arm64-<版本>-unsigned.ipa`，需要自行签名后安装。

如需本地签名 IPA，同时设置 `READWISE_IOS_CODESIGN_IDENTITY`（钥匙串已有身份）、`READWISE_IOS_PROFILE`（mobileprovision 绝对路径），可选 `READWISE_IOS_BUNDLE_ID`（默认 org.ohmycloud.readwise）。脚本校验 profile 有效期和 App ID，使用 profile 的 entitlements 签名。设备必须满足 profile 的分发与设备限制；此流程不创建 App Store archive 或上传 TestFlight。CI 默认不接触 Apple 证书。

默认 Windows 未签名，macOS 临时签名且未经公证，Android 使用 Makepad 开发证书；这些是测试分发包。正式分发签名参数见 README 中既有平台打包说明。Android 开发证书不能作为长期升级身份，生产发布前应固定私有 keystore。Linux 包未捆绑系统动态库，基线为 Ubuntu 22.04。

本机只能直接验收 macOS、Android 和 iOS 构建；Windows、Linux 与 Intel Mac 需对应 runner 实际构建并验收。Actions 成功不代表已完成各系统的安装和交互验收。

Mac 架构使用 GitHub 官方列出的 `macos-14`（arm64）和 `macos-15-intel`（Intel），见 [GitHub runner 参考](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。
