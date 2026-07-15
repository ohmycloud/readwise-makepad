# ReadWise (Makepad 重写版)

用 [Makepad 2.0](https://zhanghandong.github.io/makepad-2.0-book/)（Splash DSL + 纯 Rust）重写的 ReadWise 阅读器。
原 Tauri + React 版本保留在 `../readwise`，业务逻辑将逐步从那边的 Rust 代码迁移过来。

- 里程碑规划：[docs/MILESTONES.md](docs/MILESTONES.md)
- 开发日志：[docs/DEVLOG.md](docs/DEVLOG.md)

## 工程结构

```
readwise-makepad/
├── app/          # Makepad UI 应用（当前为 M0 技术验证 demo）
└── docs/         # 里程碑与开发日志
```

后续将新增 `core/`（从 readwise 的 src-tauri 迁移的纯业务逻辑 crate）。

## 运行（桌面）

```bash
cargo run -p readwise-app            # 开发构建
cargo run -p readwise-app -- --hot   # 热重载：改 script_mod! 内的 Splash 代码即时生效
cargo run -p readwise-app --release  # 发布构建
```

依赖 makepad dev 分支（git 依赖），首次构建会拉取并编译 makepad，耗时较长属正常。

## 运行（iOS，M0 验证项）

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

## M0 验证清单（当前阶段）

启动后应看到一个模拟 EPUB 章节，请逐项检查：

1. 中文是否正常显示（若为方块/豆腐字，说明需要注册中文字体，反馈给 AI 处理）
2. 标题层级、加粗斜体、引用、列表是否有排版区分
3. `<ruby>` 注音是否渲染（预期大概率不支持，会退化成普通文本——这正是要确认的）
4. 长文滚动是否流畅（40 段正文）
5. 点击"重新渲染"按钮，右上角计数是否变化（事件链路验证）
6. `--hot` 模式下改 `app/src/app.rs` 里 Splash 部分（如 title 文本）保存后是否即时生效
7. iOS 模拟器/真机能否跑起来
