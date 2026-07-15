# iOS 真机部署避坑指南（Makepad + cargo-makepad）

> 基于 2026-07-14 首次部署 iPhone 17（iOS 26.5）的完整踩坑记录。
> 按此清单顺序操作可避开我们踩过的全部坑。

## 核心概念（先读，能少走一半弯路）

部署真机需要**三方一致**，任何一处对不上都会失败，且报错信息极具误导性：

```
Bundle ID（--org + --app 拼出）
   = provisioning profile 里登记的 App ID
   且 profile 的设备列表包含你手机的硬件 UDID
   且用 profile 对应团队的证书签名
```

- Bundle ID = `{--org}.{--app}`，例如 `--org=com.ohmycloudy --app=readwise` → `com.ohmycloudy.readwise`
- 手机有**两个 ID**：CoreDevice UUID（`xcrun devicectl list devices` 显示，形如 `29836C5B-...`，给 `--device` 用）和硬件 UDID（形如 `00008150-...`，profile 里登记的是它）。别搞混。
- 免费 Apple ID 的 profile **7 天过期**，过期后重跑一次 run-device 续签。

## 部署清单（按序执行）

### 0. 环境卫生检查（我们在这浪费的时间最多）

```bash
which -a rustc cargo   # 只应看到 ~/.cargo/bin 一套；若 /opt/homebrew/bin 在前 → 冲突
```

⚠️ **坑 1：Homebrew Rust 劫持**。若 brew 装过 rust 且 PATH 靠前，`rustup run stable cargo` 的 cargo 会调到 Homebrew 的 rustc，报 `E0463 can't find crate for core/std`——即使你已 `rustup target add`。报错提示让你装 target，完全是误导。
解法：`brew uninstall rust`（先 `brew uses --installed rust` 确认无依赖）或把 `~/.cargo/bin` 提到 PATH 最前。

### 1. 一次性安装

```bash
cargo install --git https://github.com/makepad/makepad --branch dev cargo-makepad
cargo makepad apple ios install-toolchain
```

### 2. 先过模拟器（无需签名，验证编译链路）

```bash
open -a Simulator   # ⚠️ 坑 2：模拟器必须先启动，否则报 "No devices are booted"
cargo makepad apple ios --org=com.ohmycloudy --app=readwise run-sim -p readwise-app --release
```

⚠️ **坑 3：选项位置**。`--org/--app/--profile/--cert/--device` 必须写在 `run-sim`/`run-device` **之前**，写在后面直接报缺参。
⚠️ 模拟器卡顿正常（GPU 软件转译），真机流畅得多，别在模拟器上下性能结论。

### 3. 手机准备

1. 设置 → 隐私与安全性 → 开发者模式 → 开启 → 重启。
2. USB 连 Mac，弹窗点"信任"。
3. **让 Xcode 完成设备开发准备**：Xcode → Window → Devices and Simulators → 选中手机 → 等 "Preparing device for development" 走完。
   ⚠️ **坑 4：no DDI**。跳过这步的话 `xcrun devicectl list devices` 里设备状态是 `connected (no DDI)`，devicectl 安装必败。确认状态变成纯 `connected` 再继续。

### 4. 生成签名三件套（借 Xcode 之手，免费账号可用）

Xcode 新建空 iOS App 工程：
- Bundle Identifier 填 **将来命令行要用的那个**（如 `com.ohmycloudy.readwise`）——⚠️ **坑 5：这里填的 ID 和 `--org`.`--app` 拼出来的必须一字不差**，我们曾因 `com.ohmycloudy.readwise.ReadWise` vs `org.ohmycloud.readwise` 折腾了三轮。
- Team 选个人团队，勾 Automatically manage signing。
- **运行目标选真机**（不是模拟器！）⌘R 跑一次到 App 在手机上打开为止。目的是让 Xcode 生成含本机 UDID 的 profile。

### 5. 查签名参数

```bash
cargo makepad apple list        # ⚠️ 坑 6：是 apple list，不是 apple ios list
security find-identity -v -p codesigning   # 证书哈希
ls ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/   # profile 文件
```

验证 profile 内容是否匹配（App ID、设备 UDID、有效期）：

```bash
security cms -D -i <profile路径> | plutil -extract Entitlements.application-identifier raw -o - -
security cms -D -i <profile路径> | plutil -extract ProvisionedDevices.0 raw -o - -
```

⚠️ **坑 7：多份相似 profile**。机器上可能积累多份名字相近的 profile，`--profile` 传**文件绝对路径**最保险，别传名字。

### 6. 部署

```bash
cargo makepad apple ios \
  --org=com.ohmycloudy --app=readwise \
  --profile="<profile绝对路径>" \
  --cert=<证书哈希> \
  --device=<CoreDevice UUID> \
  run-device -p readwise-app --release
```

⚠️ **坑 8：报错被吞**。cargo-makepad 对 devicectl 失败只打一句 `devicectl failed, falling back to ios-deploy...` 就丢弃真实原因，而 ios-deploy 不支持 iOS 17+，必然跟着失败。**看到这句就手动跑 devicectl 拿真实报错**：

```bash
xcrun devicectl device install app --device <CoreDevice UUID> \
  target/apple/makepad-apple-app/aarch64-apple-ios/release/<crate名>.app
```

常见真实报错对照：
| 报错 | 原因 |
|------|------|
| `0xe8008012 profile cannot be installed on this device` | profile 不含这台设备的 UDID（步骤 4 没在这台真机上跑过） |
| `0xe8008015 valid provisioning profile not found` | Bundle ID 与 profile 的 App ID 不一致（坑 5） |
| `connected (no DDI)` 状态下失败 | 坑 4，回步骤 3.3 |

### 7. 手机端首次启动（两道关，均正常现象）

1. **信任开发者**：设置 → 通用 → VPN 与设备管理 → 开发者 APP → 信任。
2. **在线验证**："无法验证 App，需要互联网连接"——⚠️ **坑 9**：这是手机连 Apple 服务器（ppq.apple.com）的**全自动校验，不是人工审核**。失败几乎都是网络问题：切蜂窝数据（最有效）、关 VPN、确认时间"自动设置"、开关飞行模式、重启。验证通过后 7 天内不再询问。

## 快速续签（7 天过期后）

重跑步骤 6 的命令即可；若 profile 也过期了，先重做步骤 4（Xcode ⌘R 一次）刷新 profile。
