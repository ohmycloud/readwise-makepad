# ReadWise Makepad 重写 — 开发日志

> 约定：每次开发会话在此追加一节，记录做了什么、遇到的问题、决策及理由。倒序排列（最新在上）。

## 2026-08-31 — 书架列表卡片按内容测量高度

- 书架列表卡片固定为 94px，但内部已有书名、作者、阅读进度和标签四行；字体实际行高、三段间距与上下 padding 相加后可能超过固定高度。越界的“在读 · xx%”仍会绘制，却被 PortalList 中下一张不透明卡片的背景覆盖，因此看起来只剩部分文字。
- 卡片改为 `height: Fit`，由 Makepad layout turtle 按四行内容和 padding 计算真实高度，再由 PortalList 记录项目尺寸。长书名、不同平台字体度量或中英文文案不再依赖猜测出来的固定行高，相邻卡片的 10px 间距保持不变。

## 2026-08-31 — 全应用页面统一阅读场景配色

- 用户复测发现只有阅读页跟随纸张背景，书库、AI 对话、AI 技能、生词本、统计和设置仍透出 Makepad 默认灰色。根因是页面切换逻辑刻意让非阅读页恢复应用主题，并且这些页面的大量卡片和控件继续读取灰色 `theme.*` token。
- “阅读背景”现提升为整个应用场景的唯一配色来源：七个页面、桌面/移动导航和系统标题栏共享相同 surface；纸白、米黄、浅绿使用浅色控件语义，夜间使用深色控件语义，“跟随主题”保留用户选择的明暗主题。
- 新增场景主题映射，将卡片、输入框、按钮、文字、图标、选择态和统计图 token 一并投影到同一 `ReaderScenePalette`。切换背景会触发 ScriptReapply，动态列表模板也不会在滚动复用后恢复灰色；`main_column` 改为独立绘制批次的实色容器，透明页面不再漏出窗口底色。
- `cargo check --workspace --offline` 与 App 回归 29/29 通过；桌面 App 实际启动并完成数据库、设置和书架恢复，无 Splash/Widget 运行时错误，待用户逐页视觉验收。

## 2026-08-30 — M3 阅读器目录恢复左对齐

- 阅读器目录项复用了 `ButtonFlat`，同时把按钮宽度设为 `Fill`；Makepad 的 `ButtonFlat` 默认 `align: Center`，因此章节标题会在整行居中。这与 EPUB 表格的居中模板无关，是目录模板一直没有覆盖按钮默认布局。
- 目录按钮现显式使用水平靠左、垂直居中，并让 label 占满按钮可用宽度；改动仅作用于目录项，不影响普通按钮和正文表格。
- `cargo check --workspace --offline`、App 回归 29/29 与 `git diff --check` 通过；桌面 App 实际打开已导入 EPUB，无 Splash/Widget 运行时错误，Android arm64 release APK 重建成功。

## 2026-08-30 — M3 EPUB 表格标题与响应式居中

- Makepad `Html` 已支持 `table`，但不提供 `caption/figcaption` 的块级布局；旧清洗管线去掉这些标签外壳后，标题文字会直接粘在表格前的同一个 TextFlow 块里，使表格从残留文本位置起排，看起来整体右移并损失可用宽度。
- 导入管线现在把 table 内的 caption 提升为表格前的独立加粗段落，把 figcaption 转成独立说明段，并在 `split_blocks` 对顶层 table 做防御性边界切分。标题和表格因此分别参与 PortalList 虚拟化，不再共享布局状态。
- 阅读器新增普通/注音两种表格模板：表格在可用宽度内以 960pt 为桌面上限居中，窄屏自动填满剩余宽度，单元格继续由 Makepad 原生表格布局换行并完整展示。手机正文左右内边距从 32pt 收窄到对称的 12pt；表头底色和边框也投影到当前阅读纸张色板。
- 新增 caption、figcaption、残留前导文本和表格块识别回归；完整回归 74/74 与 `cargo check --workspace --offline` 通过，桌面 App 实际启动无 Splash/Widget 错误，Android arm64 release APK 重建成功，待真实 EPUB 在桌面和 Android 上复测。

## 2026-08-30 — M3 技术书序号与章节原图查看

- 技术书常用小 PNG 表示代码 callout 序号。旧管线把所有 `<img>` 都实例化为 `Fill × 320` 的块级图片，因此 1/2/3 这类十几像素的标记会被暴力放大并严重模糊。现在解析器只在 `pre/code` 语境中识别数字 `alt/title` 或 callout/number 文件名，转成清晰可选中的文本 `[1]`；普通插图不会被误转换。
- 开启注音后的列表降级也修正了：`ul` 仍用项目符号，`ol` 保留 `start` 语义并生成真正的文本编号，不再把有序列表全部拍成圆点。
- 正文插图改为按源图像素、设备 DPI、可用宽度和 520pt 高度上限等比缩小，绝不超过一个源像素对一个物理像素的清晰尺寸。点击/轻触图片会通过 Makepad widget action 打开跟随阅读背景的全屏原图窗口，显示原始像素尺寸；滚动拖动超过 8pt 会取消点击，避免手机滚动误弹窗。
- 新增回归覆盖大小图 DPI 缩放/长图高度限制、有序列表起始值、代码 callout 的 `alt`/文件名/资源缺失路径及普通插图不误转换。`cargo check --workspace --offline`、`git diff --check` 和完整回归 70/70 通过，桌面 App 实际启动无 Splash/Widget 错误，Android arm64 release APK 重建成功。

## 2026-08-30 — 手机横屏保持移动端单栏布局

- 原响应式判定只检查窗口宽度：手机旋转到横屏后长边通常超过 720pt，因此会被误当成桌面端，显示占位的左侧栏并挤压阅读区。
- 现在使用平台感知的纯几何判定：手机竖屏继续按宽度进入紧凑模式，iOS/Android 横屏在短边小于 600pt 时也保持底部导航与单栏阅读；平板和桌面端仍使用侧栏布局。
- 回归测试覆盖手机横竖屏、平板横竖屏、桌面端矮窗口与未知几何尺寸，防止后续又退化为“只看长边”。
- `cargo check --workspace --offline`、`git diff --check` 与完整回归测试 62/62 通过；Android arm64 release APK 已重建，`versionCode=2`，待真机验收横屏的底部导航、单栏阅读和书架状态。

## 2026-08-30 — M8 性能第三轮：长章节 PortalList 投影缓存

- 第二轮只缓存了注音后的整段 HTML，每次旋转恢复、阅读设置刷新或高亮更新仍会对长章节重复执行 `split_blocks`。现在惰性缓存直接改为 PortalList 需要的章节块，后续刷新只克隆已切块投影并注入当前高亮，不再重跑 HTML 分块器。
- 这个结构没有同时保留“注音 HTML + 注音块”两份长文本：缓存由整段字符串改为分块向量，内存量级不变。标记已认识会更新已缓存块，未访问章节仍在首次生成时查询 `known_words`。
- 新增 512 段合成长章节回归，锁定首次分块后的缓存复用；另覆盖返回副本不能污染缓存、已缓存/未缓存章节的已认识词处理。`cargo check --workspace --offline`、`git diff --check` 和完整回归 65/65 通过，Android arm64 release APK 重建成功，桌面 App 实际启动无 Splash/Widget 运行时错误；真实长书的 GPU 滚动体感仍保留为最终人工验收。

## 2026-08-30 — Android 横竖屏状态保持（待真机复测）

- 用户真机复测发现，旋转到横屏后书架和当前阅读状态丢失，界面回到样例文章并停留在“数据库初始化中”。根因是部分 Android 设备在旋转时除了 `orientation|screenSize` 还会报告密度、屏幕布局、最小屏幕尺寸或 UI mode 变更；原清单没有声明处理这些变更，Activity 可能被重建。
- Android 清单现完整接管影响窗口排版的配置变更，正常旋转只投递 `WindowGeomChange` 并重算响应式布局，不再销毁 Makepad 运行时。
- 同时增加防御性恢复：如果 OEM 仍重建应用界面树，新 App 会立即从进程内状态重投影书架、当前页、当前书籍、章节与可见段落；数据库已就绪时直接发送现有快照，不重复打开 SQLite。阅读页在没有真实当前书籍时强制回到书库，避免把内置样例误当用户内容。
- 第二轮真机复测表明数据未丢失：点击“全部标签”即可恢复书籍。这定位到从紧凑布局切换到横屏布局时，原先隐藏的 Makepad 子树才初次实例化，因而暂时显示 DSL 中的“数据库初始化中/书架空”默认值。现在会先让 Window 完成几何事件与控件实例化，再从集中状态重投影数据库就绪信息、当前页、书架数据、筛选摘要与空状态显隐；进入书库也会主动执行同一投影。
- 第三轮复测说明几何事件结束时投影仍然太早：`BookShelf` 原本在 DSL 中默认隐藏，而 `WidgetRef::set_visible` 对尚未实例化的引用会直接无效。现将数据所有者 `BookShelf/PortalList` 始终挂载，其 `draw_walk` 每帧从 `SHELF_VISIBLE` 读取真实数据；“书架空”只作为独立覆盖层。旋转时等完成第一次 Draw、确保子树实例化后再投影状态并请求第二帧，不再依赖标签点击产生的延后 action。
- 测试 APK 的 Android `versionCode` 从 1 提升到 2，避免手机文件管理器或安装器对同文件名、同版本包产生缓存/辨识混淆；覆盖安装保留原有沙箱数据。
- Android 书架加载后的提示文案同步改为“从系统文件选择器导入”，不再显示桌面端拖放指引。
- `cargo check --workspace --offline`、`git diff --check` 与完整回归测试 62/62 通过；桌面实际启动无 Splash/PortalList 运行时错误；新 arm64 release APK 待 Android 真机再次验收往返横竖屏。

## 2026-08-30 — M7 Android EPUB 系统选择器（真机通过）

- 用户真机安装后没有看到“导入 EPUB”，直接原因是按钮的 Splash 静态显隐条件只包含 macOS/iOS，Android 点击处理也没有平台分支。现在 Android 与 iOS/macOS 一样在书库右上角显示入口，空状态和进度/取消/错误文案也走同一条状态投影。
- 实现 Android Storage Access Framework 多选：系统 `ACTION_OPEN_DOCUMENT` 返回 content URI 后，应用 Activity 把文件流复制到私有 cache，JNI 回调用 `Cx::post_action` 投递给 Makepad UI，再复用现有后台 EPUB 解析、数据库入库和书架刷新链路；成功后清理 cache 副本。不申请存储、媒体或“访问全部文件”权限。
- 当前锁定 Makepad 的 `MakepadActivity` 已出现 `onActivityResult` 预留位，但 `cargo-makepad` 仍只编译固定 Java 源与自动生成的空子类，没有应用级 Java 注入出口。因此打包脚本先生成标准类，再编译仓库内的 `MakepadApp` 子类并让第二次打包复用该 class；不修改 Cargo 缓存中的 Makepad 源码，也不复制整份框架 Activity。
- 真实 arm64 release APK 重建成功；`javap`/DEX 检查确认 `openEpubPicker` 与 `onActivityResult` 存在，`llvm-nm` 确认 Rust 动态库导出匹配的 JNI 符号。Android 真机导入主链路已通过用户验证；本轮加入横竖屏修复后，完整回归测试 60/60 通过。

## 2026-08-29 — M3 阅读场景视觉统一（待实测）

- 根因是阅读背景之前只投影到 `reader_surface` 和正文颜色，标题栏、目录、笔记、阅读 AI、输入框、选区菜单与词卡仍使用 Makepad 应用级灰色 theme token，两个并行主题域把阅读页割裂成灰色 UI 和纸张正文。
- 新增集中的 `ReaderScenePalette`：继续以 `reader_palette` 作为纸张/正文的唯一真值；桌面/移动导航、顶部状态栏、正文与左右功能面板全部直接使用这个纸张底色，不再派生不同明度的 chrome/panel 背景。只为 field、border、muted 和 accent 交互元素派生语义色，让按钮、输入框与边框承担层级区分。
- 复测指出最左侧导航轨仍是一个不必要的灰色例外。现在桌面侧边导航、移动端安全区与底部导航都由页面状态投影：进入阅读器时加入同一 `ReaderScenePalette`，离开时恢复应用深/浅色系，既没有局部例外也不会把书籍纸色泄漏到书库和设置页。
- macOS 红黄绿窗口按钮所在的最上层不是应用内标题栏，而是 Makepad `Window` 内部的 `main_window.caption_bar`。该容器及其 `ReadWise` 标题文字现在也纳入同一页面配色投影，并使用独立 draw batch 避免不透明纸色遮住标题。
- 回归测试锁定场景 surface/text 必须复用核心阅读色板，并要求导航只在阅读页加入纸张配色，防止再次分叉或跨页污染。`cargo check --workspace --offline`、完整测试 59/59、`rustfmt --check` 和 `git diff --check` 通过；桌面 App 真实启动无 Splash 组件类型/颜色属性解析错误。

## 2026-08-29 — M3 代码块背景跟随阅读纸张（待实测）

- 代码块首版为了直接沿用 Makepad CodeView 的深色 token 色板，将容器背景写死为 `#20252b`，因此它绕过了阅读器纸张设置，在纸白、米黄和浅绿背景上都变成突兀的黑色块。
- 现在每种代码模板都与 `reader_palette` 使用同一纸张底色，用 1px 低对比边框表达代码区域；纸白/米黄/浅绿/跟随浅色使用新的浅色语法色板，夜间/跟随深色保留高对比深色色板。切换阅读背景时 PortalList 依据模板 ID 替换已复用的卡片，不会残留旧配色。
- `cargo check --workspace --offline`、完整测试 57/57、`rustfmt --check` 和 `git diff --check` 通过；桌面 App 真实启动无 Splash 模板/属性解析错误。

## 2026-08-29 — M3 EPUB 代码块格式修复（待实测）

- 问题根因是 EPUB 消毒会保留 `pre/code` 的语义标签，但会剔除出版方的 CSS 和包裹语法 token 的 `span`；Makepad `Html` 只能把剩余内容当普通等宽文本绘制，因此着色、缩进和显式换行会丢失或混在一起。
- 章节分块后现在会识别顶层 `pre`，剔除纯展示用的内层标签，解码 HTML 实体，同时保留源码空格、缩进、换行和注释文本。恢复后的源码使用 Makepad 原生只读 `CodeView` 绘制，在深色代码面板上重新进行常用语法着色；不执行 EPUB 中的 CSS 或脚本。
- 代码文本提取管线同步把 `<br>` 恢复为换行，避免复制或选区文本再次黏连。代码块识别结果按章节缓存，PortalList 滚动时不重复解析 HTML。
- 新增测试覆盖 token 标签、CRLF/`br`、缩进、字符实体、注释和持久化高亮私有元数据。`cargo check --workspace --offline`、完整测试 57/57 通过；桌面 App 真实启动无 Splash 属性、数据库或设置恢复错误。

## 2026-08-29 — M8 阅读器纸张背景

- 设置页新增独立于应用深浅主题的“阅读背景”：纸白、米黄、浅绿、夜间与跟随主题。新安装及尚未保存该键的旧数据默认使用纸白，解决正文只能透出应用灰底的问题。
- 每种纸张色绑定经过配对的正文色；背景应用在正文列表外层，正文色则在 `ChapterList` 绑定每个 PortalList 可见段落时重新投影，避免滚动复用后局部恢复旧颜色。主题切换、ScriptReapply、窄屏侧栏切换和启动恢复仍走同一 `ReaderSettings` 状态链路。
- 新增稳定存储值与损坏值回退测试，数据库使用 `reader.background` 与其他阅读设置在同一事务内保存。`cargo check --workspace --offline` 通过；完整测试在允许回环端口的环境 56/56 通过；桌面 App 真实启动无 Splash 解析或设置恢复错误。

## 2026-08-29 — M7 Android release APK 与 Play Store 打包入口（待真机）

- 将 `readwise-app` 的 Makepad 依赖从根 workspace 继承改为 app crate 直接声明同一 git 分支：当前 `cargo-makepad` 会把 app manifest 复制为独立 Android wrapper workspace，原来的 `workspace = true` 在包装后没有 `[workspace.dependencies]` 可继承，真实构建会在解析 manifest 时失败；Cargo.lock 仍固定为同一 Makepad commit。
- 修正跨平台入口：桌面/iOS 的 stderr logger 与 `main() -> app_main()` 不再参与 Android/OHOS library 构建，由 Makepad 导出的 JNI 原生入口接管。新增 Cargo 产品元数据（ReadWise / `org.ohmycloud.readwise` / versionCode 1 / minSdk 26）、五档 Android launcher icon，以及项目专用 `AndroidManifest.xml.template`。
- 默认 Makepad 清单会申请相机、录音、定位、蓝牙、媒体存储和查询全部应用等 ReadWise 不需要的权限，现缩减为 `INTERNET` 与 `ACCESS_NETWORK_STATE`；同时关闭系统备份，保留 Makepad Activity 的主题、配置切换和单实例语义。
- 新增 `scripts/package_android.sh`：默认生成开发证书签名的 arm64 release APK；只有显式提供发布 keystore 与密码时才额外生成 Play Store AAB，密码走环境变量，ABI、SDK 路径和递增 versionCode 均可配置。项目含原生 TLS 依赖，工具链安装必须带 `--full-ndk`；实测还发现旧 `cargo-makepad` 的精简 NDK 目录复制缺陷，因此 README 明确要求 CLI 与锁定 Makepad 对齐。
- macOS arm64 上已真实生成 43MB `ReadWise-android-aarch64-0.1.0.apk`。`aapt` 验证包名、versionCode/versionName、minSdk 26、targetSdk 35、五档图标和仅两项网络权限；`apksigner` 验证 v2/v3 签名有效且证书为 Android Debug。AAB 发布签名与 Android 真机安装/交互仍待对应凭据和设备。

## 2026-08-29 — M8 Windows/Linux 桌面分发流程（待原生环境实跑）

- 基于当前锁定的 `cargo-makepad desktop build` 实际行为补齐另外两个桌面平台：Linux 不再分发 Makepad 自动生成、指向开发机 `target` 绝对路径的 `.desktop`，而是输出按架构和 Cargo 版本命名的 tar.gz，包含便携可执行文件、产品图标和无需 root 的 XDG 用户安装器。
- Windows 新增 PowerShell 发布脚本，输出版本化 ZIP；沿用 Makepad 构建期 ICO 资源嵌入，并支持通过 `READWISE_WINDOWS_CERT_THUMBPRINT` 选择系统证书进行 Authenticode SHA-256 签名。时间戳 URL 由发布环境显式传入，签名后由系统 API 复验，证书及服务配置不写入仓库。
- 两个脚本都拒绝在非目标宿主系统运行，避免把交叉编译或错误 ABI 的文件误标为正式包；使用 `--locked --offline` 固定依赖。Bash 语法、PowerShell AST、平台保护分支和 `git diff --check` 已在 macOS 验证；最终二进制、桌面菜单集成、SmartScreen/Authenticode 仍待对应 Linux/Windows 原生主机实跑。

## 2026-08-29 — M8 macOS 公证发布闭环（待凭据实跑）

- `scripts/package_macos.sh` 新增可选 `READWISE_NOTARY_PROFILE`：Developer ID 签名并生成上传 ZIP 后，使用钥匙串中的 `notarytool` profile 提交且等待结果，再依次执行票据装订/验证、签名复验和 Gatekeeper `spctl` 评估，最后重新归档，确保交付 ZIP 中包含已装订的 `.app`。
- 公证 profile 与 ad-hoc 签名组合会在构建前明确拒绝，缺少 Xcode `notarytool` 也会提前失败；只提供 Developer ID 而不提供 profile 时仍可产出签名但未公证的包，默认无凭据路径继续生成本机 ad-hoc 包。
- README 补充 `notarytool store-credentials` 的一次性钥匙串配置和完整发布命令。凭据不会进入环境默认值、日志文件或仓库；实际 Apple 服务提交仍需 Developer ID、Team ID 与 App Store Connect 专用密码。
- 验证：`bash -n` 通过，ad-hoc + notary profile 的无效组合在编译前被拒绝；本机 Xcode 16 的 `notarytool 1.1.2`/`stapler` 参数与脚本一致。无凭据离线路径真实重建 16MB `.app` 与 6.2MB ZIP，plist、ad-hoc 签名和归档内容均通过；实际 Apple 公证服务调用仍待发布凭据。

## 2026-08-29 — M8 macOS 原生 EPUB 选择器（待实测）

- 补齐桌面书库只能拖放或使用投递箱的体验缺口：macOS 现在显示“导入 EPUB”按钮，点击后打开原生 `NSOpenPanel`，支持一次选择多本 EPUB，并复用现有后台导入、错误汇总和书架投影刷新链路。
- 首轮把按钮在 DSL 中静态设为隐藏，再尝试于 `Event::Startup` 调用 `set_visible(true)`；用户实测按钮没有出现。按当前 Makepad 故障排查记录修正为在 `script_mod!` 初始对象中用 Rust 平台布尔插值决定可见性，避免 Startup 设置被脚本应用/重应用覆盖；本地启动读取到 widget 实际 `visible=true`，书库加载后绘制矩形为 75.72×22 px，确认没有被布局裁掉。同时把早期 demo 遗留的默认“阅读”页改为书库页，Rust `AppPage` 与 Splash `PageFlip` 初始状态保持一致，启动后可直接看到导入入口。
- 当前 Makepad macOS `SelectFileDialog` 仍只有日志占位；实现直接复用框架已经链接的 Apple Objective-C runtime，不引入会冲突的第二套 AppKit/应用事件循环。iOS 继续使用已有 `UIDocumentPicker`，Windows/Linux 保持拖放与 import 目录。
- macOS 选择的是用户原文件，因此导入成功后只保留应用数据目录内的副本，绝不清理源文件；iOS import 模式产生的应用私有临时副本仍按原语义清理。`cargo check --workspace --offline`、完整测试 55/55、iOS 交叉检查、桌面构建/启动和 `git diff --check` 均通过；Linux 交叉检查只因本机未安装 `aarch64-linux-gnu-gcc` 停在 SQLite C 编译器探测。待人工验证取消、中文路径、多选、重复书籍与源文件保留。

## 2026-08-29 — M8 书籍删除防误触（待实测）

- 书架删除从卡片上的即时执行改为明确的二次确认，弹窗展示书名，并说明应用内阅读进度、笔记和高亮会一并删除，应用外的原始 EPUB 不受影响。
- 删除状态不再复用普通书架加载结果：确认后显示进行中，成功后刷新当前标签投影并显示剩余本数；数据库删除或随后刷新失败都会直接反馈在书库页，不再只写日志后伪装成功。
- 自动化验证：`cargo check --workspace --offline`、完整测试 55/55、iOS 目标交叉检查、`git diff --check` 与桌面启动冒烟均通过；过程中未操作用户书库中的真实书籍。待人工验证取消、点击弹窗外关闭、确认删除，以及中英文提示。

## 2026-08-29 — M8 macOS 发布包第一轮

- 核对当前锁定的 `cargo-makepad` 源码而非沿用旧 CLI 经验：`desktop build` 能生成 macOS `.app` 并支持 codesign，但只有 app crate 同时提供 32/64/128 PNG 与 ICO 时才进入桌面资源和 bundle 后处理；默认 plist 还会把 Bundle ID 固定为 `dev.makepad.*`、版本固定为 `1.0.0`。
- 从项目 1024px RGB 源图机械生成完整桌面 PNG 尺寸与多尺寸 ICO，放入 `app/resources`，Windows 链接资源、Linux desktop entry 和 macOS bundle 现在共用同一套产品图标；没有手工维护多份视觉源文件。
- 新增 `scripts/package_macos.sh`：锁定依赖并离线构建 release，复制为 `ReadWise.app`，写入 `org.ohmycloud.readwise`、Cargo 版本、教育分类和高 DPI 标记，校验 plist/签名并输出版本化 ZIP。默认 ad-hoc 签名供本机验证；设置 `READWISE_CODESIGN_IDENTITY` 时改用 Developer ID、hardened runtime 与时间戳，凭据不入库。
- 真实生成的 arm64 App 为 16MB、ZIP 为 6.2MB；plist、ad-hoc 签名、Designated Requirement 与归档目录均验证通过，通过 LaunchServices 启动 `.app` 成功。公开下载仍需 Apple Developer ID 公证/装订；这些操作需要外部证书和公证凭据，留到正式发布阶段。
- README 已从 M0 demo 文案更新为当前 workspace 结构、macOS 打包/签名说明和现行 iOS 入口。

## 2026-08-29 — M8 性能第二轮：章节注音惰性缓存与启动基线

- 打开 EPUB 的旧路径会在后台串行给整本书的所有章节注音，完成前 UI 收不到 `ReaderAction::Opened`；长书即使用户只读当前章，也会承担全书字符串扫描、HTML 重写和额外内存。现保留消毒后的原始章节，只为恢复章节立即生成注音，其余章节第一次访问时生成并缓存。
- 关闭注音直接投影原始章节，重新开启只生成当前章，不再重新解析整本 EPUB；标记已认识只修改已经生成的缓存，并把 lemma 写入当前书的 known 集，未访问章节以后生成时会自然跳过。原始 HTML 始终不被运行时 ruby 破坏。
- 新增 EPUB 打开阶段日志，分别记录解析与首章注音耗时；应用启动记录数据库/设置及书库投影就绪时间。本机真实启动基线为 258ms / 850ms，方便后续对构建模式、数据库体积和设备做同口径比较。
- 审计全部 `new_batch`：均位于带背景且包含文本的卡片、面板或弹窗，符合 Makepad 绘制顺序要求；没有发现无意义的 DrawList。滚动正文/列表和聊天属于高频动态区域，不适合纹理缓存；动态集合由 Rust + PortalList 持有，没有 Splash 动态对象持续增长证据，因此继续使用自动 GC，不加入不可逆静态标记或周期性强制 GC。
- 新增测试覆盖只缓存访问章节、关闭注音仍返回原始 HTML、未缓存章节跳过新标记的已认识词。完整测试 55/55、iOS 目标交叉编译、`git diff --check` 和桌面启动冒烟均通过；待长书翻章与注音开关人工体验后关闭性能项。

## 2026-08-29 — M8 书库标签与筛选（待实测）

- 补齐 M2 延后的 tags 主链路：书架卡片展示标签并提供编辑弹窗，支持逗号/中文逗号/分号分隔、清空标签、去空白、大小写不敏感去重，以及每书 8 个、每项 24 字符的边界校验。
- 标签以 JSON 保存回 `books.tags`，同时幂等补齐全局 `tags` 目录；更新使用事务，书籍不存在会明确失败。书架查询统一反序列化标签，旧记录、NULL 或损坏 JSON 安全降级为空列表。
- 书库新增标签下拉筛选、显示数量摘要和独立的“没有匹配的书籍”空状态。筛选列表沿用版本化 `ListProjection`，每次数据集合替换会重置 PortalList 并切换等价模板，避免旧书卡残留、闪现或筛选按钮与卡片索引错配。
- 首次运行冒烟发现把 `Modal` 放在 `Root` 层会缺失 Window 的绘制 pass 并触发 Makepad `pass_stack.last().unwrap()`；现将 Window body 改为 Overlay，由共同祖先持有内容和弹窗。修正后 App 持续运行无 Splash/绘制崩溃。
- 验证：`cargo check --workspace --offline`、完整测试 53/53、`git diff --check` 与桌面 App 启动冒烟均通过。待人工验证标签新增/删除、筛选切换、筛选为空及中英文切换后关闭 milestone。

## 2026-08-29 — M8 中英文界面完成

- 应用设置新增中文/English 语言选择，并纳入 ReaderSettings 事务化持久化；启动恢复、损坏值回退与主题 ScriptReapply 后的状态恢复使用同一强类型语言值。
- 侧栏与移动导航、书库、阅读器、AI、技能、生词本、复习、统计和设置页的固定文案及主要动态状态统一接入集中式 i18n 映射；PortalList 卡片在绘制时读取当前语言，切换后无需重载业务数据。
- i18n 阶段已提交为 `dc7f3d2 feat: add persistent Chinese and English UI`；M8 i18n 项关闭。

## 2026-08-28 — M8 旧数据库迁移闭环

- 审计发现 `CREATE TABLE IF NOT EXISTS` 只能创建缺失表，不能给已有表补列；原有兼容代码每次启动盲试三条 `ALTER TABLE`，再依赖 SQLite 英文错误文本中的 `duplicate column name` 判断是否已迁移，既没有显式版本，也无法发现其他残缺结构。
- 引入 SQLite `PRAGMA user_version`，当前 schema 定为 v2（旧 Tauri 基线为 v1；v2 包含 `app_settings` 与 `book_notes` 的 section/normalized range）。版本为 0/旧版的数据库原位升级；高于应用支持版本的数据库会在执行 schema 写入前拒绝打开，防止旧应用误改新库。
- 迁移改为先读取 `PRAGMA table_info`，只补确实缺失的 `section_id`、`norm_start`、`norm_end`，重复执行幂等且不依赖错误文案。迁移后逐一验证 15 张业务表及其运行时必需列，问题在启动阶段给出具体表/列，而不是等用户进入某页面才出现 SQL 错误。
- SQLite 连接统一启用 `foreign_keys`，使 schema 已声明的书籍删除级联/笔记置空约束在连接池的每条连接上真正生效。
- 新增旧版 `book_notes` 内存库回归：插入旧数据后连续执行两次升级，确认三列补齐、原行保留且版本为 v2；另测未来 v999 数据库在任何建表前被拒绝。完整测试 48/48 与 `cargo check --workspace --offline` 已通过；真实数据库从 v0 启动升级到 v2，无需补列且 App 正常运行，升级后的 `foreign_key_check` 无违规记录。

## 2026-08-28 — M8 性能第一轮：正文与 AI 流式热路径

- 阅读正文的 PortalList 此前在每次绘制可见项时都调用 `Html::set_text`，即使 item 仍绑定同一段落也会重新解析整段 HTML。现先比较 Html 已持有的 body，仅在 PortalList item 确实改绑到其他段落时解析；滚动停稳后的普通重绘不再重复做解析工作。
- 正文字号、字体、行距、选区色和选区范围改为每次 `ChapterList::draw_walk` 只读取一次全局状态。字号与选区色直接写入公开的 `TextFlow`/`DrawColor` 字段，移除每个可见段落两次 `script_apply_eval!`，避免高频绘制反复进入 Splash VM；默认字号仍来自当前主题 token，深浅主题切换行为不变。
- AI SSE 一次事件中可能包含多个文本片段，原实现逐片段加锁、更新状态并重绘整棵 App UI。现合并同批文本后只写入和重绘一次，重绘范围只覆盖主 AI 与阅读侧栏的两个 ChatList；状态 Label 继续由自身更新，不牵连书架、正文和其他页面。
- 本轮没有盲目添加 `texture_caching` 或手动 GC：正文/列表是持续变化的滚动内容，不适合整树纹理缓存；GC 与 draw batching 需要 Studio 数据再决定，避免用额外显存和 draw list 换取未经测量的“优化”。
- 验证：`cargo check --workspace --offline`、`cargo test --workspace --offline`（46/46）、`git diff --check` 与桌面 App 真实启动均通过；无编译告警（依赖仓库重复 package 提示除外）、Splash 解析错误或启动异常。

## 2026-08-28 — M8 第三刀：正文字体与行距

- 设置页补齐正文字体和行距：字体提供跟随主题正文样式的“标准”与适合代码类书籍的“等宽”；只影响 EPUB 正文及 ruby 主词，不改变应用 UI 和注音释义字体。当前锁定的 Makepad 未内置适合英语长文的衬线字体，因此没有用不可靠的系统字体路径伪造一个选项。
- 行距提供“默认 / 舒适 / 宽松”三档。普通段落分别使用 1.2/1.45/1.75，带生词注音的段落则使用 2.2/2.45/2.75，始终保留额外垂直空间，避免用户调整后注音与相邻行重叠。
- 字体 family 与行距在 `ChapterList` 每次绑定 PortalList 可见项时重新投影到 Html 的 normal/italic/bold/bold-italic/fixed 样式，避免虚拟列表复用旧卡导致设置只影响部分段落。两项均进入 `ReaderSettings` 的事务化存储、启动恢复和损坏值回退链路。
- 新增稳定存储值和注音行距安全边界测试；`cargo check --workspace --offline`、`cargo test --workspace --offline`（46/46）、`git diff --check` 与桌面 App 真实启动均通过，启动日志无脚本解析或设置恢复错误。

## 2026-08-28 — M8 第二刀：主题 token 迁移与运行时切换

- 没有直接添加一个只改变系统按钮的“浅色模式”开关。审计发现 App 自定义 UI 仍有大量固定深灰背景和浅灰文字；运行时切换 Makepad 主题只会让官方 widget 与未覆盖属性变化，书架卡片、阅读侧栏、聊天气泡、统计卡等仍会保持深色，形成不可用的混合主题。
- 将书架/笔记/技能/聊天/生词虚拟列表模板、阅读目录/笔记/AI 面板、移动安全区与底栏、统计卡/图表、正文链接，以及页面级主文字/次要文字/强调色迁移到 `theme.color_bg_*`、`theme.color_text*`、`theme.color_label_inner_inactive` 和 `theme.color_highlight`。用户明确选择的注音、选区和高亮色仍保留固定预设，不被主题偷换。
- 用户复测发现统计柱体被通用 `theme.color_highlight` 映射成灰色。图表颜色不再借用交互强调 token：新增 `color_chart_reading`、`color_chart_vocab` 和 `color_chart_grid` 应用级语义 token，深色主题恢复蓝/青绿柱体，浅色主题使用对浅背景对比更清晰的深蓝/深青绿。
- PortalList 目录项此前在 Rust 重绑时硬编码选中/普通背景；改为 `TocList` 的 live 主题色字段再通过插值应用，避免列表复用绕过主题。色板未选中底色同样提升为 App live 主题字段，选中格才使用用户选择色。
- 设置页新增“深色 / 浅色”单选项并纳入现有 `ReaderSettings` 事务化持久化。切页状态提升为 Rust `AppPage`，主题重新应用后会恢复当前 PageFlip 页面、响应式布局、阅读侧栏、书架/笔记空状态、生词投影与聊天控制状态，不把 UI 默认值误当业务状态。
- 首次实现使用 `request_live_edit()` 重跑整棵脚本树，真实浅色→深色测试捕获到 Makepad ScriptObject use-after-free。最终改为保留深浅主题共享对象模板，运行时用 `splat` 原地更新当前主题，再走框架专为共享对象变更提供的 `request_script_reapply()`；它不会销毁 widget 树，也避免重置输入和动态列表。
- 验证：`cargo check --workspace --offline`、`git diff --check` 通过；沙箱外完整 `cargo test --workspace --offline` 44/44 通过。桌面 App 深色正常启动，并用测试态浅色启动后由数据库恢复深色，实际走完主题 ScriptReapply，持续运行无崩溃或脚本错误；视觉细节仍需人工验收。

## 2026-08-28 — M8 第一刀：阅读设置统一持久化

- 审计确认设置页的正文字号、生词注音开关/字号/字色/背景、词字色/背景、选区色和新高亮色此前都只写全局内存，重启必然恢复默认。新增强类型 `ReaderSettings`，为数值和色槽提供默认值、有限值检查、范围裁剪与损坏数据回退。
- 复用现有 `app_settings` 表，以 `reader.*` 键在一个事务中读取/保存全部阅读设置。数据库初始化完成时随 `DbAction::Ready` 一次性把设置交回 UI，早于书架加载，避免“先按默认注音打开书、稍后才恢复开关”的闪烁和竞态。
- 所有阅读设置入口改为修改后自动后台保存，设置页显示保存中/成功/失败状态。连续快速点击采用递增版本和串行写锁：排队的旧快照会跳过，正在写的旧快照之后仍会由最新版本覆盖，不会出现 A+/色板连点后数据库落到中间值。
- 恢复设置时统一更新全局渲染状态、CheckBox、字号标签和六组色板；若持久化状态关闭注音，会同步剥除内存章节的 ruby。字体族和深浅主题仍留在 M8 后续。
- 验证：`cargo check --workspace --offline`、`cargo test --workspace --offline`（43/43）和 `git diff --check` 通过；按用户要求未启动 App 做人工验证。

## 2026-08-28 — M7 第三刀：移动虚拟键盘与 Android 导入边界

- 接入 Makepad 顶层 `VirtualKeyboardEvent`，由 App 统一持有键盘可见状态：iOS 在 `WillShow`、Android 在 `DidShow` 阶段隐藏窄屏底部导航，`DidHide` 后再恢复，避免底栏与软键盘争抢可用高度。窗口尺寸/安全区变化也复用同一可见性判定，不会在键盘仍打开时误把导航重新显示出来。
- 键盘出现时取消正在跟踪的阅读横滑，键盘保持打开期间禁止启动和提交翻屏手势，避免在输入、候选或拖动光标时意外翻页。状态只保留在 App 层，不复制到各个 TextInput 和页面。
- 修正窄屏文本选择模型：Makepad 当前 `PortalList selectable` 在触屏按下时立刻开始选择，会抢走正常纵向滚动，而项目保存完整选文和弹出操作条的镜像此前又只处理鼠标。窄屏现关闭框架的“触下即选”，正文保持正常拖动滚动；收到平台长按事件后按英文单词/单个中日韩字符建立选区，长按未松手时可继续拖动跨块扩选，松手后复用现有复制、笔记、生词、翻译、问 AI、发音和高亮菜单。桌面仍走原 PortalList 跨块选择链路。
- 审计 Android 系统文档选择器链路：当前锁定的 Makepad `MakepadActivity.onActivityResult` 是空实现，且 `cargo makepad android` 只编译固定框架 Java 源与生成的 `MakepadApp`，没有项目级 Java 回调注入点。因此仅从 Rust 启动 `ACTION_OPEN_DOCUMENT` 无法取回 URI；暂不复制整份框架 Activity 形成高风险分叉，等待上游提供 activity-result 桥接或升级到包含该能力的版本。
- 本轮按用户要求不启动 App 做手工验证；新增底栏可见性与长按词界回归测试，`cargo check --workspace --offline` 和 `cargo test --workspace --offline`（42/42）通过。设备端键盘动画、触摸手感与焦点行为留待后续验收。

## 2026-08-28 — M7 第二刀：触摸翻屏、iOS Files 导入与原生沙箱目录

- 窄屏阅读正文新增单指横向翻屏：左滑下一屏、右滑上一屏，复用桌面快捷键已有的 `page_scroll`，因此章内翻屏、跨章、进度标签和持久化仍走唯一链路。手势要求短时且水平位移显著；纵向滚动、多指、长按、已有选区以及目录/笔记/阅读 AI 面板都会取消，避免破坏滚动和文本选择。
- 修正移动数据目录来源：不再假设 iOS/Android 进程一定设置 HOME/XDG。启动时从 Makepad `Cx::get_data_dir()` 取得 iOS Application Support 或 Android `Activity.getFilesDir()`，再交给 `readwise-core` 创建稳定的 `ReadWise` 子目录；桌面仍使用原有 `dirs::data_dir()/ReadWise`，现有数据库位置不变。
- 审计确认当前锁定的 Makepad 文件对话框不可复用：macOS select-file 仍是日志空壳，iOS 对 `SelectFileDialog` 直接报未实现。为 iOS 在应用 crate 内补了最小 UIKit `UIDocumentPickerViewController` 桥接：书库显示“导入 EPUB”，从 Files 按 EPUB UTI 多选，回调进入现有后台导入/书架刷新链路，成功后清理 iOS import 模式产生的私有临时副本；桌面拖入的用户原文件仍绝不删除。Android 系统选择器仍待接入。
- 验证：新增水平/垂直/超时手势判定及平台沙箱路径测试；`cargo check --workspace --offline`、完整 iOS 目标 `cargo build -p readwise-app --target aarch64-apple-ios --offline`（含链接）、`cargo test --workspace --offline`（40/40）、桌面 App 启动冒烟和 `git diff --check` 均通过；桌面日志确认仍使用原有 Application Support 数据库。Files 回调、触摸手感与长按互斥仍需真机验收。

## 2026-08-28 — M7 第一刀：窄屏导航、单栏阅读与移动生命周期

- 保留唯一一棵 PageFlip/业务 widget 树，在其外重组为“桌面侧栏 + 主列（页面 + 移动底栏）”，避免用两套响应式页面复制 TextInput、PortalList 和聊天状态。窗口宽度低于 720pt 时隐藏桌面侧栏，显示可横向滑动的完整底部导航；桌面窗口缩放与手机横竖屏均走同一尺寸事件。
- 接入 `WindowGeomChange` 的安全区：窄屏顶部为刘海/Dynamic Island 留白，底部导航叠加 Home 指示条 inset；未知的启动尺寸不会误切移动布局。
- 阅读页在窄屏采用真正单栏：目录、笔记、阅读 AI 打开时占满正文宽度并隐藏章节列表，关闭后恢复正文；三个面板互斥，避免 260/320/360px 固定侧栏把手机正文挤没。选区操作条改为可横向滚动，完整保留复制、笔记、生词、翻译、问 AI、发音和高亮。
- 补齐移动后台/恢复计时：Background/Pause 立即结束当前阅读会话，Foreground/Resume 回到阅读页再创建新会话；异步创建尚未返回时会标记失效，返回后先结束旧会话，避免切后台时间被统计为阅读。
- 验证：新增窄屏断点和异步会话失效测试；`cargo test --workspace --offline` 38/38、桌面默认 1080×720 与临时 430×800 两种运行时启动、`cargo check --workspace --target aarch64-apple-ios --offline`、`git diff --check` 均通过。真实 iPhone 的安全区、触摸与键盘体验仍待用户验收。

## 2026-08-28 — PortalList 空状态与缓存失效统一治理

- 用户实测发现：保存第一条笔记后，右侧笔记栏仍出现多张“还没有笔记”卡片。根因与此前生词本问题相同：`entries.len().max(1)` 把空状态伪造成虚拟列表数据，PortalList 在数据范围变化和 item 复用时会继续请求旧位置，越界分支又把每个旧位置绘制成占位卡。
- 不再对单个页面打补丁：抽出通用 `ListProjection<T>`，用 revision 标记数据集合替换；笔记、生词和技能列表在 revision 变化时重置首项并切换两套等价模板，使相同 item id 的旧 widget 无法污染新集合。越界 item 一律跳过，不再具有 UI 语义。
- 建立统一约束：PortalList 的 item range 必须严格等于真实数据条数，空状态必须位于列表外。笔记和书架改为独立空状态视图；目录、技能移除伪占位条目，同时审计并清除了业务列表中的 `len().max(1)`/固定单条占位范围。
- 新增回归测试覆盖初始空投影、列表从多条缩为一条时旧 item id 失效、由非空变空时绝不生成占位 entry。验证：`cargo check --workspace --offline`、`cargo test --workspace --offline`（36/36）、`aarch64-apple-ios` 交叉编译、桌面 App 启动与 `git diff --check` 均通过。

## 2026-08-28 — M6 第二刀：阅读时长与词汇统计

- 复用 schema 已预留但从未写入的 `reading_sessions`，补齐会话开始、30 秒 checkpoint、离开阅读页/退出应用结算和异常退出恢复；计时只覆盖用户实际停留在阅读页的时间，切到书库、AI、生词本、统计或设置页都会停止。
- 新增统计聚合模块与同步 `Db` 门面：累计/今日阅读时长、阅读天数、书籍完成数、生词学习/掌握数、累计复习数，以及按本地日期补齐的最近 7 天阅读数据。
- 侧栏新增“统计”页：四个摘要卡、最近 7 天阅读时长图和词汇学习图；柱状图由自定义 `StatsBars` widget 使用 Makepad `DrawColor` GPU shader 批量绘制，页面数据加载与会话结算继续放在后台线程。
- 快速切页时处理“计时会话仍在后台创建”的竞态：若用户已进入统计页，会先结束刚创建的会话再查询，避免刷新早于结算；重复结束为幂等操作。
- 验证：新增内存 SQLite 测试覆盖会话生命周期与 7 天/词汇聚合；`cargo test --workspace --offline` 33/33、`cargo check --workspace --target aarch64-apple-ios --offline`、桌面 App 启动和放宽既有 lint 后的严格 clippy 均通过；用户已完成实测，M6 正式关闭。

## 2026-08-27 — M6 第一刀：生词本与复习闭环

- 用户连续复测定位 PortalList 缓存边界：固定位置 ID 会留下上一个筛选的旧词卡；改用非零、不重叠 ID 区间虽能隔离旧卡，却触发 PortalList 从非零起点向前补齐视口，表现为新卡闪现后消失、搜索框连续重绘时才可见。最终恢复官方稳定模式 `range_start = 0`，每次投影 revision 变化时在两套等价卡片模板间切换；相同位置因 template ID 改变会被 PortalList 强制替换，同时保持正常滚动和按钮事件索引。
- 状态筛选控件由三个互不关联的普通按钮改为原生 `RadioButtonSet`，将唯一选中索引映射回 App 筛选状态，每次刷新都同步控件选中态。
- 用户实测补修：原实现用 `entries.len().max(1)` 把空状态伪造成 PortalList 第 0 条词卡，且筛选范围缩小时没有重置虚拟列表的 `first_id/scroll`，导致占位卡与旧词卡残留、输入和“已掌握”筛选看似不刷新。现将空状态移到列表外独立展示；每次搜索/筛选投影变化都重置 PortalList 到首项，并同步重画列表与页面布局。
- 生词本页从“启动时拼接前 50 条文字”升级为 PortalList 虚拟化词卡：显示表面词/原形、释义、例句、词级、学习状态和复习次数；阅读页新增生词后立即刷新，不再要求重启。
- 新增实时搜索和“全部/学习中/已掌握”筛选；每条词卡支持系统发音、单词复习、切换学习状态与移除。完整词汇数据和筛选/复习会话由 App 统一持有，PortalList 只读取投影，避免业务状态散落在复用的列表 item 中。
- 新增队列复习模式：默认按当前“学习中”词汇开始，也可从单条词卡进入；先展示单词和例句，主动显示释义后选择“继续学习”或“认识了”，数据库原子更新状态、复习次数和时间，并自动进入下一词。
- 数据层补齐不存在记录的明确错误，并新增“复习计数 + 状态”单条 SQL 原子更新；从普通文本选区加入且释义为空时，会自动回查离线词典释义。
- 验证：用户已完成生词本列表、双向状态筛选和持续显示复测；`cargo check --workspace --offline`、`aarch64-apple-ios` 交叉编译、桌面运行时启动与 `cargo test --workspace --offline`（31/31）通过。新增测试覆盖筛选映射/投影替换、重复收藏、复习计数/状态更新、非法状态和不存在记录；M6 生词本项关闭，下一步进入统计页。

## 2026-08-27 — M5 第三刀：桌面 llama.cpp 本地推理

- 桌面设置页新增可选本地 AI：配置 `llama-server` 可执行文件、GGUF 模型、端口、上下文、线程数、GPU 层数与自动启动；配置复用 `app_settings` 持久化。应用不擅自下载大体积二进制或模型，由用户明确选择本机文件。
- 新增 `LocalAiManager` 统一持有子进程：仅监听 `127.0.0.1`，启动时检查模型与端口，读取 stdout/stderr 判断就绪并轮询 `/health`，保留最近日志用于错误反馈；支持 180 秒超时、加载中取消、停止、异常退出检测和应用关闭清理。
- 本地服务就绪后，现有 OpenAI-compatible 流式聊天自动切到 `http://127.0.0.1:<port>/v1/chat/completions` 与固定模型别名；停止或异常退出后恢复用户原有远程 API 设置，不分叉聊天和 side-chat 的请求链路。
- 子进程管理只在 macOS/Windows/Linux 编译；移动端保留同一份配置类型但隐藏本地服务 UI，并明确只走远程 API。`aarch64-apple-ios` 交叉编译已通过。
- 验证：`cargo check --workspace --offline` 通过；提升沙箱网络权限后 `cargo test --workspace --offline` 27/27 通过，其中假 `llama-server` 覆盖了启动、就绪、轮询与停止生命周期；桌面 App 启动无 Splash/DSL 错误。当前机器未安装 `llama-server` 且没有 GGUF，真实模型加载与推理仍保留为 M5 验收项。

## 2026-08-27 — M5 第二刀：AI skills / Prompt 模板管理

- 新增完整 skills 数据层与同步 `Db` 门面：列表、创建、编辑、启停、删除，统一校验空名称/空内容与重名；系统技能允许编辑内容和启停，但名称固定且不可删除，避免重启时因改名又补入一份默认模板。
- 默认 skills 初始化从“仅新数据库写入”改为每次启动 `INSERT OR IGNORE` 幂等补齐：不覆盖用户已有内容，同时以后新增内置模板也能进入旧数据库。
- 新增“AI 技能”页面：左侧 PortalList 展示系统/自定义模板、启停与操作，右侧为可滚动的多行 Markdown Prompt 编辑器；支持新建、编辑、清空和删除，所有数据库操作继续放在后台线程，通过 action 回 UI。
- 技能进入真实推理链路：启用的系统技能自动成为所有聊天的 system prompt；启用的自定义技能可点“下次提问使用”，切到 AI 对话页后只增强下一条请求，聊天气泡与持久化历史仍保留用户原文，避免把内部模板暴露进会话记录。
- 用户已完成真实端点复测，M5 的主 AI 流式 chat 与阅读 side-chat 正式勾选通过；skills 实现后 M5 仅剩桌面本地推理与移动端远程策略收口。
- 验证：`cargo check --workspace --offline`、`cargo test --workspace --offline`（24/24）与 `git diff --check` 通过；桌面 App 真实启动，旧数据库 schema/default skills 初始化和 Makepad Splash 运行均无错误。

## 2026-08-27 — M5 第一刀：真实 AI 流式对话 + 阅读选区问 AI

- 用户实测补修：阅读页顶层快捷键原先无视键盘焦点，右侧 AI、笔记、主 AI 页和设置页的 `TextInput` 输入空格/方向键时会同时驱动正文翻页；现仅在所有文本输入框均未聚焦时响应阅读快捷键。
- macOS 流式无回复根因：Makepad 的 `NSURLSession` 流式 chunk/complete 事件按设计携带 `status_code = 0`（该回调无状态码），旧兼容层误判成 HTTP 失败并在第一个 chunk 取消请求；现只判定非零的非 2xx 状态码。另加入跨任意网络/UTF-8 分片的 SSE 增量缓冲，兼容 DeepSeek 先发 `reasoning_content` 再发 `content`，思考阶段给出状态反馈但不展示内部推理；普通 JSON 错误、网络错误、空回答和 60 秒超时均显示在主/侧 AI 状态栏。
- 用户实测补修：选区菜单原本只是视觉浮在正文上方，点击仍会穿透到 `ChapterList`；正文的原始 `MouseDown` 抢先清空 `SELECTED_TEXT`，随后“问 AI”等按钮动作只能读到空文本。顶层现在按菜单实际区域捕获一次完整指针手势，正文列表在按下、移动、抬起阶段不再接收穿透事件；“问 AI”会保留选文、打开右侧栏并提交提示，同时消除复制、笔记、生词、翻译、发音和高亮的同类隐患。
- 用户实测补修：阅读选区“问 AI”改为留在阅读页并打开 360px 右侧对话栏，主 AI 页与侧栏共享消息、流式状态、停止/新对话和持久化线程。
- 无回复根因：设置保存的是 SDK 语义的 DeepSeek 根地址 `https://api.deepseek.com`，而 `makepad_ai` 把配置当完整 POST URL；现按服务商规则归一到 `/chat/completions`（同时兼容常见 `/v1` 与完整 URL）。并在官方后端外补齐 HTTP 非 2xx、非 SSE 200、空响应检测，错误不再静默挂起。
- 用官方 `makepad_ai::OpenAiBackend` 替换 M0 的模拟打字机，复用 Makepad 原生网络事件处理 OpenAI-compatible SSE；支持多轮上下文、停止生成、错误反馈和新对话。
- 新增 AI 对话页与 PortalList 消息气泡；设置页可配置完整 chat-completions 端点、模型名和 API Key（本地免鉴权服务允许空 key），密钥输入默认遮罩。
- 迁移 threads 数据层：强类型消息 JSON、最近全局线程恢复、upsert 保存；新增 `app_settings` 表持久化 AI 配置。回答完成、停止和请求失败均保存当前会话。
- 阅读选区浮层新增“问 AI”：把完整跨块选区组装为阅读解释提示，直接切到 AI 页并发起流式请求。
- M4 经用户实测通过，milestone 正式关闭。M5 的 chat/side-chat 代码完成，等待用户用真实端点验证后勾选。
- 验证：`cargo check --workspace --offline`、`cargo test --workspace --offline`（18/18）、`cargo clippy --workspace --all-targets --offline`、`git diff --check` 均通过；桌面 App 真实启动且 schema 迁移成功，无 Splash/DSL 错误。Clippy 仅保留既有 EPUB/TOC/RubyWord lint。

## 2026-08-26 — M4 实测修复：注音空库、滚动划词、选区操作浮层

- 注音未显示的直接原因不是设置开关，而是当前 `word_glosses/book_glosses/known_words` 均为空，旧实现对此静默跳过。加入 ECDICT MIT 许可派生的离线基础释义：按项目现有词级表筛选 2-4 级词、压成单条短中文释义，共 4437 条/约 95 KB；新安装或近乎空库启动时 `INSERT OR IGNORE` 补齐，保留用户与书籍语境释义。来源版本、许可与可复现生成脚本一并入库。当前实测库已从 0 补到 4437 条。
- 选区不再把完整文本寄托于 PortalList 当前仍持有的可见 widget：新增与 TextFlow 相同字符空间的 `rendered_text/rendered_text_for_range`，跨块、离屏回收后仍能导出完整引用；PortalList 每次滚动/复用 item 时从全局范围重新投影选中态。
- 修复滚轮延伸：Makepad PortalList 遇“滚轮事件且未检测到首鼠标键”会结束内部选择；ChapterList 现在继续持有手势，在滚动后按新视口命中/估算光标块并重画选区，MouseUp 才真正结束。
- 原底部普通流 `sel_bar` 改成阅读正文共同祖先持有的 Overlay 浮层，避免被页面剩余高度/批次遮住。操作包含复制、笔记、单词加入生词本、浏览器翻译、系统 TTS 发音、高亮、关闭，并显示选中文本预览与动作反馈。
- 验证：`cargo check --workspace`、`cargo test --workspace`（16/16）、`git diff --check` 通过；`cargo run -p readwise-app` 正常启动、Splash/DSL 无报错，并打印 `Initialized 4437 starter glosses`。严格 clippy 仍被既有 `epub_meta.rs` 两条 lint，以及既有 TOC/RubyWord 的 type-complexity/collapsible lint 拦截；临时允许这些既有规则后本次代码通过。

## 2026-08-26 — M4 高亮收口 + notepad 主链路

- 修复高亮按钮实际无法保存：`ChapterList` 原先收到窗口内任意 `MouseDown` 都先清空 `SEL_RANGE`，点击列表外的“高亮/添加笔记”按钮也会误清；现在只有按下点位于 PortalList 区域内才重置选区。
- 选区引用：仍用自跟踪镜像保存区间，同时从 PortalList 公开的 `get_selected_text()` 取得纯文本，供 notepad 使用；切章、取消、保存成功统一清理原生选区与镜像状态。
- 永久高亮改为逐条颜色：`inject_colored_marks` 把颜色名与正文一起编码进 `<mark>`/`<ruby>` 内文（`U+E001` 分隔），MarkSpan 与 RubyWord 每次 `set_text` 重绑颜色，避免 Html 自定义 widget 复用导致颜色快照串位；选区覆盖注音词时按整词着色。历史高亮不再跟随当前设置色整体变色。
- 迁移独立 `notes` 数据层（与旧版语义兼容）：新增 create/update/list-by-book/delete 同步门面与 Unicode/书籍元信息/CRUD 测试。
- 阅读页新增右侧笔记栏：选中文本→“添加笔记”→引用预览 + 可选补充→保存；按当前书倒序列出笔记并支持删除。笔记栏由阅读页共同祖先持有，避免把跨列表的业务状态塞进段落 widget。
- 验证：`cargo test --workspace` 15/15 通过；`cargo run -p readwise-app` 启动成功，无 Splash 注册/解析错误。`cargo clippy --workspace --all-targets -- -D warnings` 仍被既有 `epub_meta.rs` 两条 lint（相同 if 分支、test module 后还有 item）拦截，本次未扩大无关 diff。
- 待实测：拖选后分别点“高亮/添加笔记”；跨段引用；重启后高亮颜色与笔记仍在；删除笔记；高亮上的选区/点词穿透。

## 2026-07-17 — M4 收官刀：永久高亮标注

- 存储：book_notes（type=annotation/style=highlight/color；section_id=章下标；norm_start/end=块下标×1e6+字符下标——老 schema 原生三列正好够用）。core：add_highlight / get_highlights_by_chapter + Db 包装。
- 选区区间获取：PortalList 未暴露 range API（只有 has/clear_selection）→ ChapterList 用**公开接口自跟踪镜像**（原始 Mouse 事件 + get_item/selection_point_to_char_index 命中；绘制仍由原生选区负责），SEL_RANGE 静态区共享。已记 TODO：给 makepad 上游提 selection_range() PR。
- 渲染：`inject_marks`（core，12 测试含跨标签拆段）按"绘制字符空间"把区间包成 `<mark>` 段（与选区同坐标系：标签不计/实体计1/ruby 只计词部）；MarkSpan widget（TextFlowLink 样板：手写 WidgetNode、point_hits_area=false 不挡选区、荧光底用上一帧区域铺、颜色每帧读 HL_SLOT）。
- UI：选完弹底部 sel_bar（高亮/取消）；保存后入 CURRENT_BOOK.highlights + 当前章原地重建（build_chapter_chunks 统一注入路径，MarkedKnown/注音开关刷新也走它）；设置页"高亮颜色"色板（黄/青/蓝/橙）。
- v1 已知限制（更新）：① 多色高亮已于 2026-08-26 修复；② 空白折叠可能造成字符偏移漂移（TextFlow 折叠连续空白，注入器按原文计数）——实测偏了再做 sanitize 期空白归一；③ 高亮记录暂未存 text 列（notepad 已单独保存引用文本）。
- 待验证：拖选→高亮→重启还在；跨段高亮；高亮上的选区/点词穿透；颜色切换即时变色。

## 2026-07-17 — 注音总开关（用户需求）

- 设置页顶部 CheckBox"生词注音"；勾选才显示注音字号/字色/背景/词字色/词背景五行配置（set_visible 联动），未勾选原样显示文章。
- 切换即时生效：关 → `remove_all_ruby` 原地剥全部注音（不重载不跳转，复用原地消注机制）；开 → 重开当前书走注音管线。spawn_open_book 注音段受 GLOSS_ENABLED 门控。
- 默认开启（保持现有阅读流；默认值偏好待 M8 设置持久化时定）。11 测试过。

## 2026-07-17 — 选区两 bug：颜色路径 + 手势仲裁

- 颜色不生效：选区真正的绘制者是 TextFlow 的 `draw_selection`（DrawQuad，theme.color_u_3 灰），不是 `draw_block.selection_color`（块级样式）。apply 改 `draw_selection.color`，alpha 66→88 提高可见度（灰色半透明在深底上近乎不可见，可能也是"只能选几个字"观感的一部分）。
- 手势仲裁失灵：PortalList 的官方仲裁（按在交互件→子件点击；按普通文本→列表接管选区并屏蔽子件）依赖 `WidgetRef::find_widgets_from_point` 的自报机制——**自报条件是 `point_hits_area`**，而 RubyWord 用默认实现（基于从未赋值的 area 字段，恒 false）→ 仲裁器看不见注音词。修复：RubyWord 手写 `WidgetNode`（derive 从 Widget 改为 WidgetRef/WidgetSet/WidgetRegister，照 TextFlowLink 样板），`point_hits_area` 遍历 drawn_areas。
- 附带清理：#[redraw]/#[area]/#[walk]/#[layout] 是 Widget 全量派生的属性，手写 WidgetNode 后改 `#[rust] area: Area`。
- **教训：凡 TextFlow 内联的交互 widget，必须手写 WidgetNode 并用 drawn_areas 实现 point_hits_area，否则选区仲裁失灵（TextFlowLink 是官方样板）。**

## 2026-07-17 — M4 文本选择 + 选中颜色配置

- 文本选择走原生机制：Para/GlossPara 的 Html `selectable: true` + ChapterList 的 PortalList `selectable: true`（跨块选择由 PortalList 统一接管，官方注释确认）；Cmd+C 复制由 TextFlow 内置（Hit::TextCopy → selected_text）。
- 选中颜色：设置页"选中颜色"色板（青/蓝/橙/黄，半透明 66 alpha），SELECTION_SLOT 槽位静态区，ChapterList 绑定时按槽位 apply `selection_color`（TextFlow #[live] 字段，字面量枚举——apply 无 prelude 教训沿用）。
- 提交 e0f2？（上一刀已交：M4释义卡片与注音串位修复）。

## 2026-07-17 — M4 释义卡片

- 点注音词 → 阅读页底部滑出卡片（词 + 释义 + 加入生词本 + 标为已认识 + 关闭），替代原状态栏一闪。
- 加入生词本：lemma/word/gloss/level(词级表)/book_id 入 vocab_book，反馈"已加入 ✓ / 已有"。
- **严重 bug（用户发现）：标记已认识后释义错位到下一个词**。根因 = Html 自定义 widget 按序号缓存复用：`set_text`（词）每帧刷新，但释义存 widget 状态（创建时读 t 属性一次）——内容删减后 widget 与 ruby 对位偏移，旧释义挂到新词上。根治：**释义编码进 ruby 内文**（`词 RUBY_SEP 释义`，U+E000 私有区分隔符），set_text 每帧同时刷新词+释义，widget 变无状态绑定。**教训：可复用 widget 的一切展示状态必须从 set_text/绑定数据流入，不能依赖创建时快照。** t 属性路径保留为 demo 兼容。
- 标为已认识（用户否决重开方案——影响阅读体验）：改为**原地消注** `remove_ruby_for_lemma`——内存里剥掉该词的 `<ruby>`（lemma 匹配），块数不变、滚动位置不动、无重载闪烁；DB 写入照旧，下次开书注音管线自然跳过。**教训：能原地变换就不要重建，阅读场景对连续性极敏感。**
- CURRENT_ITEM 静态区记住在读书架项供重开；卡片状态存 App.card (word, lemma, gloss)。
- GitHub 推送待用户 gh auth login 后执行（gh 未登录、SSH 无钥）。
- 待验证：卡片弹出/关闭、加生词本反馈与生词本页计数、已认识后重注音消词。

## 2026-07-17 — 注音遮挡定案 ✅ + 注音外观全套设置

- **遮挡最终根因（调试可视化定案）**：红块（注音摆放盒）本身被切 → 裁剪者是 Html/TextFlow 自身的绘制区域；外层留白无效（clip 边界跟着下移）。修复 = padding 挪进 Html 内部（首行文字在 Html 矩形内下移，注音不越界）。叠加此前修的字盒高度低估（字号≠字盒，差 ~1.4×）。用户确认 OK。
- 色板选中态：格子底色=预设色本身（色样+选中态双重作用），`script_apply_eval` 的 `vec4(#(r),...)` 插值可用；启动时 DbAction::Ready 后刷一次初始选中态。
- 注音外观四项可配：注音字色（青/蓝/橙/灰）、注音背景（无/深灰/深青/深蓝）、词字色（默认/青/蓝/橙，经 tf.font_colors push/pop——HtmlLink 同款）、词背景（用上一帧 drawn_areas 铺底，首帧延迟一帧）。
- 会话级配置，持久化到 settings 表留 M8。
- 坑：`script_apply_eval!` 片段是裸作用域，**没有 prelude——`vec4()` 等函数不可用**（运行时 call target NotFound），但 `#x` 颜色字面量是解析器原生类型可用。动态颜色 → 改为逐字面量枚举（paint_swatch slot 表）。

## 2026-07-17 — M3 关闭 ✅ + M4 第一刀：真书注音管线 + 老数据导入

- M3 全清单完成（用户实测图片 OK），MILESTONES 已关。
- M4 注音管线（产品灵魂功能落地）：
  - `vocab/lemma.rs`：嵌入老项目 lemmas-en.json（900KB）+ word-levels.json，`normalize_token/lemmatize/word_level` 逻辑照抄 JS 版；
  - `vocab/annotate.rs`：`annotate_html`——文本节点分词（字母开头含 '’-），lemma 在释义表且不在已认识集 → 包 `<ruby t="释义">`；跳过标签内部/code/pre/HTML 实体（&amp; 的 amp 不会被当词）；
  - `Db::gloss_context`（全局释义 + 本书语境覆盖 + known 集）；`spawn_open_book` 加载后全书注音；
  - `Db::import_legacy_vocab`：ATTACH 老库 INSERT OR IGNORE（word_glosses 4775 条/known/vocab_book，vocab_book 的 book_id 置 NULL 防 FK），设置页"导入老版词汇数据"按钮触发。
- 7 个单元测试全过（含注音只标生词、跳过 code/实体）。
- 用户点名：一次性迁移不该占设置 UI → 拆除导入按钮/Db::import_legacy_vocab/legacy_db_path，改为 sqlite3 CLI 直接迁移（已执行成功：4775/24/7 全部入新库）。
- App 内 ATTACH 失败的真因（存档）：**sqlx 连接池的 ATTACH 只对执行连接生效**，后续语句被派发到未挂载的连接 → no such table。若以后需要 ATTACH 类操作，必须用单连接（pool.acquire 同一 conn 执行整段）。
- **原则沉淀：一次性运维操作走脚本/CLI，不进产品 UI。**
- 待验证：打开英文书 → 生词自动绿字注音、点注音词状态栏显示、生词本页 7 条。
- 注音遮挡——底层根因（读 text_flow/turtle/draw_text 源码定案）：**TextFlow 换行有五条互不相通的路径，注音是排版不知情的叠加层**——①段内 wrap 行距吃 line_spacing（唯一可控好的）②段落间=paragraph_margin.bottom（可调）③标题后=heading_margin.bottom（可调）④块首行=无（靠容器 padding）⑤ **li=new_line_collapsed 硬编码零间距无旋钮**。表格 cell 行高同样不可控。
- 根治（收敛宿主路径而非打补丁）：GlossPara 设 paragraph_margin.bottom:1.4 + heading_margin.bottom:1.1 + padding-top + 四字体 line_spacing 2.0；**注音章节渲染前 `flatten_lists` 把 ul/ol/li 拍平成"• "段落**（li 无解→绕开）；**annotate 跳过 table 内部**（与 code/pre 同级规则）。9 个单元测试锁行为。
- 注音字号（6-12，步进0.5）与颜色（青/蓝/橙/灰预设）进设置页；RubyWord 每帧从 GLOSS_FONT_SIZE/GLOSS_COLOR 静态区读取。
- 坑：这版 `id!()` 返回单个 LiveId（非数组），路径要 `&[id]`。
- 待验证：全书翻读注音无遮挡（重点：列表变段落后的观感、表格内无注音是否可接受）、注音字号/颜色设置联动。
- 用户暂不接受"列表转段落/表格不注音"取舍——记为待重审（正解可能是上游给 TextFlow 提 leading 支持或自研注音段落排版，M4 后评估）。
- **调试可视化揪出真凶嫌疑**：注音摆放高度此前用"字号"(7.5) 而非真实字盒（ascender-descender ≈ 1.4×），注音顶部天然伸出预留空间 → 系统性半裁。已改用排版器真实度量；并加 GLOSS_DEBUG=1 环境变量（绿=词锚定矩形、红=注音摆放盒）。
- 第二轮截图暴露三个残余缺口，逐个封死：① **li 内嵌 `<p>`**（Manning 常见结构）在 flatten 后变成 p 嵌套破坏段落间距 → flatten 剥掉 li 内的 p 标签（li_depth 跟踪，测试锁定）；② **折行单词的注音锚点错误**——词跨行时 drawn_areas[0] 是行尾小残段，注音画到了残段头上（截图"同重/表刁"悬空半裁）→ 改锚最宽的段（词主体）；③ 块首行头顶量不足 → padding-top 14→22、line_spacing 2.0→2.2（覆盖注音字号上限 12 的字盒高度）。10 测试全过。

## 2026-07-15 — M3 图片刀：EPUB 内嵌图片渲染

- core：`extract_images` 打开书时把包内 image/* 资源提取到 `books/<id>/res/`（按文件名，幂等）；`rewrite_img_src` 把章节里 `<img src>`（含 %XX 解码、按 basename 匹配）改写为提取后的绝对路径；消毒白名单加 img、split_blocks VOID 加 img。
- app：`chapter_image.rs` 新 widget，走 ruby 同款自定义标签扩展点——Html 遇 `<img>` 查 `img` 模板实例化，`on_after_new_scoped` 读 src 属性，首次 draw 时 `load_image_file_by_path`。样式：宽 Fill 高 320 定高 + ImageFit.Biggest（等比含入，M8 可做按真实宽高比自适应）。
- 坑：`use makepad_widgets::*` 会引入其 `log` 导出，与 log crate 撞名 → `::log::error!` 全限定。
- 已知限制：同名不同目录的图片会互覆（basename 匹配）；SVG 插图不支持（Image widget 不解 SVG 文件路径加载）。遇到再处理。
- 待验证：技术书插图显示、图片章节滚动性能、无图书不回归。

## 2026-07-15 — M3 TOC 刀：目录抽屉 + 章节标题

- core：`load_chapters` 返回 `ChapterInfo{title, html}`——标题取消毒后首个 h1-h3 纯文本（40 字截断），fallback"第 N 节"。没用 epub crate 的 ncx toc（NavPoint→spine 下标映射跨书不稳），标题法对技术书足够；若遇标题质量差的书再升级。
- UI：头部左侧"目录"按钮开关左侧抽屉（260px SolidView，默认 visible:false）；TocList widget（PortalList，每项 ButtonFlat）；点章跳转并收起；当前章 "▸" 高亮（CURRENT_CHAPTER AtomicUsize 共享）。
- 坑：抽屉项按钮与头部按钮同名 `toc_btn` 会被深度查找撞车 → 重命名 toc_item_btn。**教训：ids! 是全树深搜，命名必须全局唯一。**
- 待验证：目录开关、点章跳转、当前章高亮、长目录滚动。
- 修复：`▸` 字形主题字体缺失渲染成豆腐块（用户截图）→ 改颜色高亮。**教训：UI 里的装饰字符只用主题字体确有的字形，不确定就用颜色/布局表达。**
- 二次修复：设按钮 draw_text.color 会被 ButtonFlat 的 animator（hover/默认状态动画）在重绘时用主题色覆盖——高亮时有时无（翻页/新书后消失）。改为**行背景高亮**（TocItem 换 SolidView，按项 apply draw_bg.color 青绿/底色），View 背景无动画竞争。**教训：不要 apply 会被 widget animator 管理的属性（Button 的 draw_text/draw_bg），要么改 View 层属性，要么走 animator 状态。**
- 崩溃修复：多章书点目录 abort——`MTLTextureDescriptor height (21510) > 16384`。根因：Para/GlossPara 用 `CachedView`（todo 示例抄来的）把列表项渲染进离屏纹理，超长章节块超过 Metal 纹理上限直接 assert。**文本块不用纹理缓存**：模板根换普通 View（宽 Fill 高 Fit）。**教训：CachedView 只适合小而固定的项（按钮卡片类），任何高度不可控的内容（长文本/表格/代码块）严禁包 CachedView。** 若后续单块布局本身过慢，方案是 split_blocks 里对超长块二次切分，而不是回缓存。

## 2026-07-15 — 阅读页去 chrome（用户反馈：学 iBooks）

- 用户点名：字号/注音色/字体切换不该显式摆在阅读页；上一章/下一章按钮多余。
- 改：阅读页头部只剩 书名 + 状态 + 章内进度（弱化小字灰色）；字号调节移入设置页"阅读"区（A-/A+ + 当前值标签）；上/下章按钮删除——章节导航全靠键盘（空格连读自动跨章、Shift+←/→）与后续 TOC。
- **设计原则沉淀：阅读页保持沉浸，控制项进设置或走手势/键盘（iBooks 范式）。** 后续做 TOC/图片渲染时同样遵守。
- 待验证：阅读页观感、设置页字号调节联动。

## 2026-07-15 — M3 翻页刀：快捷键 + 视口翻屏 + 章内进度 + 字号

- 翻屏：按 PortalList 可见块数翻页（`first_id + visible_items - 1` 重叠一块保上下文），到底自动进下一章、到顶回上一章；`set_first_id_and_scroll` 定位。
- 快捷键（仅阅读页 + 有书打开，`on_reader_page` 标志随导航维护）：空格/PgDn/→ 下屏，PgUp/← 上屏，Shift+←/→ 跨章，Home 回章首。KeyCode 来自 makepad_studio_protocol 再导出。
- 章内进度：`chapter_pos` 标签 `第x/y章 · z%`（z=已读到块/总块），翻屏/切章/滚轮（Event::Scroll）时刷新。
- 字号：A-/A+ 按钮调 `READER_FONT_SIZE`（None=主题默认，8-20 clamp）；ChapterList 绑定时 `script_apply_eval!` 设 Html 的 `font_size`——TextFlow 的标题字号按 font_size 缩放（text_flow.rs:1193），一个旋钮全局生效。
- 已知风险：CachedView 可能缓存旧字号纹理（改字号后需滚动才刷新）——待实测，若发生则换失效策略。
- 待验证：快捷键翻屏手感、跨章、百分比、字号调节即时性。

## 2026-07-15 — 修复：中文文件名拖放导入失败

- macOS 拖放的 `DragItem::FilePath` 是 URL 百分号编码（中文/空格变 %XX），makepad 未解码直接透传 → 文件系统找不到。Drop 处理时 `percent_decode_path`（15 行零依赖）解码后再导入。
- 注意 import 投递箱目录不受影响（直接读目录，无编码问题）。

## 2026-07-15 — M3 修复：真书空白根因 = 未知标签丢内容

- 用户实测：标题/章节数(1/22)加载正确但整章空白、偶发崩溃。根因即 M0 发现的 Html widget 行为：**未知标签连内容整体跳过**（handle_custom_widget 查不到模板就 jump_to_close）——真书章节全部包在 `<section>/<div>` 里，整章被吞。
- 修复（core/epub_meta）：`sanitize_for_reader`——白名单（Html widget 的 32 个 tag，grep some_id! 确认）保留；未知标签（div/section/span/a/figure…）**去壳留瓤**；script/style/head/svg/math/rt/rp 连内容删；注释/声明剔除。`split_blocks` 按标签深度归零切块（替代 `</p>` 粗切，不再把 blockquote/表格切断——疑似崩溃源之一）。
- 3 个单元测试全过（去壳留瓤/删 script/嵌套分块）。坑：raw string 里 `href="#f1"` 的 `"#` 会终止 `r#".."#`，要用 `r##`。
- 待复测：真书章节渲染、崩溃是否消失。

## 2026-07-15 — M3 第一刀：真书阅读闭环

- core：`epub_meta::load_chapter_htmls`（spine 遍历 `get_current_str` → `extract_body` 取 body 内容，空章跳过）；`books::get_status` + Db 包装（恢复进度用）。
- app：书卡加"阅读"按钮 → `spawn_open_book` 后台加载全书章节（逐章 strip_ruby）→ `CURRENT_BOOK` 静态区 → `ReaderAction::Opened{resume_chapter}`。
- `set_chapter`：章节 HTML 按 `</p>` split_inclusive 切块喂 PortalList（保持虚拟化）、标题/页码更新、`set_first_id_and_scroll(0,0)` 回顶、后台存进度（location=章节下标，progress=章/总章）。
- 阅读页头部改为 上一章/页码/下一章；流式测试与重载示例挪到设置页"开发工具"。
- 无书打开时阅读页仍显示 M0 示例章节（兜底）。
- 待验证：点"阅读"能否渲染真书、上下章切换、重启后从上次章节恢复、书架进度百分比变化、真书里的点词（`<a>` 无处不在？不，真书没包链接——点词在真书上暂不可用，M4 注音管线接上才有）。

## 2026-07-15 — M2 完成 ✅

- 收尾三项用户实测通过（真封面/删除/投递箱），已提交 75e947f。
- 编译修复三处（新坑记录）：① Script 派生宏不认字段类型的全限定路径（`std::collections::HashMap` → 必须 use 导入短名，报 `Unexpected field form` + 连锁 E0277/E0599）② `load_image_file_by_path` 收 `&Path` 非 `&str` ③ `#[live(2.0)]` 对 f32 字段要写 `2.0f32`（future-incompat）。
- 枚举 DSL 写法：`fit: Biggest` 运行时报 type mismatch（静默回退默认值），正确为 `fit: ImageFit.Biggest`（examples/uizoo/tab_image 确认；枚举一律 `类型名.变体`，与 skills 里 `@` 前缀说法不同——`@` 是 default 声明用）。
- **M2 关闭。下一步 M3 阅读器**：点击书卡片 → 打开真实 EPUB（迁移老项目 toc_parser/sanitizer 或用 epub crate 取章节 HTML）→ M0 管线渲染 → 进度保存。

## 2026-07-15 — M2 收尾：真封面 + 删除 + 投递箱语义

- M2 第一刀用户验收通过（导入/书架/持久化），已提交 e7964ab。
- 封面：Card 封面区改 `flow: Overlay`（首字色块打底 + `Image{fit: Biggest}` 叠加）；BookShelf 加 `loaded_covers: HashMap<usize, Option<String>>`，路径变化才 `load_image_file_by_path`，无封面 `set_texture(None)` 清旧纹理（PortalList item 跨书复用防串图）。
- 删除：卡片尾部 `del_btn`，`items_with_actions` 拿 item_id → SHELF 查 book id → spawn_delete 后台删+刷新。M2 先直删不确认（书可重新导入）。
- 投递箱：scan_import_dir 成功导入后删除 import/ 源文件——否则删书后重启会被重新扫回。
- 待验证：封面真图显示、无封面书显示首字、删除即时生效、import 目录文件导入后消失。
- tags 复用评估：老项目书库 tags 仅作筛选，当前书架无筛选 UI 消费场景 → 推迟到 M8 整理（MILESTONES 已注）。

## 2026-07-15 — M2 动手：books 迁移 + epub 解析 + 书架 + 拖放导入

- 导入方式定为拖放 + 导入目录（用户拍板）。
- core/books：models（SimpleBook/BookStatus/BookShelfItem，M2 只迁书库需要的，ReadingSession/BookNote 留 M3/M4）、epub_meta（epub crate 解析 title/author/language/cover，`mdata().value` + `get_cover()`）、queries（**import_book 新架构**：Rust 侧解析元数据→copy 文件与封面到 data_dir/books/<id>/→入库，源文件不删；get_shelf_items 拼绝对路径；delete_book 删目录+级联；update_progress；scan_import_dir 扫 data_dir/import/ 按 标题+大小 去重）。Db 加 data_dir 字段 + 5 个书库方法。
- UI：BookShelf widget（deref View + PortalList 卡片列表，封面暂用书名首字色块占位）；page_library 换成书架 + 顶部提示；空态卡片。启动 DbAction::Ready 后 spawn_load_shelf（扫描导入目录 + 读书目）；拖放 Event::Drag 设 DragResponse::Copy 接受、Event::Drop 取 DragItem::FilePath 过滤 .epub → spawn_import → 刷新。ShelfAction 回传计数。
- 核对：std::sync::Mutex → response.lock().unwrap()；DragItem/DragResponse 由 makepad_widgets 可达；epub crate 纯 Rust。
- 待验证：编译；拖 epub 进窗口是否入库并显示卡片；重启后书架持久；放文件到 ~/Library/Application Support/ReadWise/import/ 重启是否收录；封面占位显示首字。**封面图真实加载（Image widget 从运行时文件路径）是下一个子任务。**

## 2026-07-15 — rfd 集成评估：可行但不推荐

- rfd 平台：Windows/macOS/Linux/WASM，**iOS/Android 不支持**——桌面用 rfd 仍要为移动端另写 UIDocumentPicker/Intent，未省移动端工作量（致命）。
- Makepad 不实现 raw-window-handle（grep 无），rfd 无法 attach 父窗口 → 只能弹独立对话框，模态/焦点体验打折。
- 桌面集成技术可行：Makepad 内部有主线程 dispatch（permission 回调用的 NSBlockOperation）+ NSApp 在跑，AsyncFileDialog 内部派发弹窗能在其 runloop 转；路径 = 后台 tokio block_on future → post_action 回 UI。但 examples 无先例，中等试错成本。
- 结论：rfd 卖点（原生对话框）只覆盖桌面，恰在移动端帮不上，性价比低。推荐仍是拖放+导入目录（桌面移动通用、零依赖、原生接线）。追求一步到位则方案四自实现（NSOpenPanel+UIDocumentPicker）留 M7。
- 待用户拍板导入方式。

## 2026-07-15 — M2 调研：文件导入技术闸门（待决策）

- 老项目 books：models（SimpleBook/BookStatus/ReadingSession/BookNote 等）+ commands 900+ 行，纯 sqlx，迁移方式同 vocab（去 tauri、共享 pool）。EPUB 元数据在老项目是 foliate-js（JS）解析后传给 save_book。
- 关键发现：`epub` crate（2.1.5，纯 Rust）能解析 title/author/章节——**替代 foliate-js 在 Rust 侧解析元数据可行**，M2 架构与老项目不同（Rust 解析而非前端）。
- **技术闸门**：Makepad dev 分支内置文件对话框是**空壳**——macos_app.rs:873 `open_select_file_dialog` 只有 `println!("open select file dialog!")`，无 NSOpenPanel、无结果回传事件，examples 无一使用。标准"按钮弹选择器"走不通。
- 备选：① 拖放（`DragItem::FilePath` 已接线，能拿路径，仅桌面）② 扫描导入目录（零依赖，桌面移动通用）③ rfd crate（跨平台但需验证与 makepad 事件循环集成）④ 自实现 NSOpenPanel/UIDocumentPicker（工作量大）。
- 导入方式 = 影响架构的决策，已抛给用户（AskUserQuestion），用户 dismiss，**暂停等指示**。解析与书架主线不受导入方式阻塞。

## 2026-07-15 — M1 收尾：图标/标题/日志/主题

- 外壳验证通过（用户截图：侧栏导航、数据库就绪）。图标：方案 A（注音书页），源文件 app/resources/icon.svg；iOS 用 app/resources/icon_1024.png（cargo-makepad 自动打包），桌面 Dock 用 workspace 根 resources/icon_{32..1024}.png（makepad-platform build.rs 构建期内嵌；首次需 `cargo clean -p makepad-platform` 触发，因目录此前不存在无监听）。
- 窗口标题 `window.title: "ReadWise"`；主题改为官方三段式显式注册（theme_mod → mod.theme = mod.themes.dark → widgets_mod），运行时深/浅切换推迟到 M8（需先把自定义色改 theme.* 变量）。
- 日志：main.rs 内置 10 行 StderrLogger 接通 log 门面（core 的 log::info 可见），零新依赖。
- **M1 完成**。下一步 M2 书库：迁移 books 模块 + epub 解析（tauri-plugin-epub），书架 UI，文件导入。

## 2026-07-15 — M1 第一刀：readwise-core + App 外壳

- 新建 `core` crate（readwise-core）：`db.rs`（老 database.rs 去 tauri 化，数据目录改由 `paths::app_data_dir()` 提供 = 系统数据目录/ReadWise；schema.sql、default-skills.json 原样拷贝）、`vocab/`（models 原样 + queries 全量迁移，签名从 `AppHandle` 改为 `&SqlitePool`，修掉老代码每次调用新建连接的低效——现在共享 pool）。
- 异步策略：`Db` 同步门面持有 current_thread tokio runtime，方法内部 `block_on`。UI 侧在后台 `std::thread` 调用，结果 `Cx::post_action` 回 UI（M0 已验证的桥）——App 里零 async 管线。
- App 外壳：左侧 SolidView 侧栏（书库/阅读/生词本/设置 ButtonFlat + 底部 db 状态）+ `PageFlip`（uizoo 示例同款，`active_page: @page_reader`，Rust 侧 `page_flip().set_active_page`）。M0 demo 整体挪入阅读页；生词本页展示 DB 实际数据（计数 + 前 50 条预览），作为 core 迁移的冒烟验证。
- 启动流：Event::Startup → spawn_db_init 线程 → Db::open（建库/建 schema/默认 skills）→ DbAction::Ready/Failed → 侧栏状态与生词本页更新。
- 待用户验证：编译、四页导航、侧栏底部显示"数据库就绪"、数据库文件落在 ~/Library/Application Support/ReadWise/database/app.db。

## 2026-07-14 — Ruby 注音验证通过 ✅ + EPUB 复用方案落档

- 第三轮用户实测通过：释义悬浮词上居中、基线完美、行距拉开、点击正常（截图确认）。
- 复用评估（翻老项目代码核实）：注音数据层全在 Rust（schema 四张表 + core/vocab + tauri-plugin-epub），复用率 ~90%；只需移植两块纯 JS——`normalizeToken/lemmatize`（~50 行查表逻辑）和 `GlossAnnotator`（DOM 注入改写为章节 HTML 字符串预处理，反而更简单）。
- 设计、三轮教训、复用映射表落档 [ruby-annotation.md](ruby-annotation.md)；遗留优化（释义居中微偏、首行间距、长释义碰撞）记录在案，非阻塞。
- 至此 M0 + Ruby 专项全部完成，重写路线的最后一个技术问号拔除。下一步 M1。

## 2026-07-14 — Ruby 第三轮：字体与行距的正确来源

- 第二轮实测：基线对齐彻底解决 ✓、点击 ✓，但释义不可见（`WARNING: encountered empty font family`）、行距未生效。
- 释义字体：自建 DrawText 的 text_style 默认无字体族，必须从主题取完整 TextStyle——`text_style: theme.font_regular{ font_size: 7.5 }`（官方 browser 示例同款写法）。
- 行距真相：TextFlow 每个 run 都会用自身 `text_style_normal/bold/italic/fixed` 字段**整体覆盖** `draw_text.text_style`（text_flow.rs:1866），所以设 `draw_text.text_style.line_spacing` 无效；正确旋钮是 Html 实例的 `text_style_normal.line_spacing`（bold 同理）。行距 scale 经 layout options 的 `line_spacing_scale` 生效（draw_text.rs:2528）。

## 2026-07-14 — Ruby 第二轮：改为 TextFlow 内联绘制 + 释义叠加

- 首版两层 Label 盒子方案失败：TextFlow 把 widget 盒子顶对齐塞进行内，释义占了正文行、原词被挤到下一行（用户截图）。
- 重写（HtmlLink 同款画法）：原词交给 `tf.draw_text` 内联绘制——基线/字体/断行与正文像素级一致；`areas_tracker` push/pop 拿到词的绘制区域，释义用自有 `DrawText.draw_abs` 叠画在首段区域上方居中（`layout()` 测宽）。命中区域改用 drawn_areas 逐段 hits（跨行断词也能点）。
- 行高：新增 `GlossPara` 模板（`draw_text.text_style.line_spacing: 2.0`），ChapterList 按内容含 `<ruby` 自动选模板，为释义留出行间空间。
- 已知边界：段落首行的释义上方是段间距，极端情况可能与上一项贴近；后续在真实阅读器里统一调段距。

## 2026-07-14 — Ruby 注音 Demo（方案 A：Html 自定义标签 widget）

- 源码确认扩展点：Html 遇未知标签走 `handle_custom_widget`（html.rs:968）——按标签名查 Html 实例的同名子模板，实例化 widget 内联进 TextFlow，属性经 `Scope::with_props_index(doc, index)` 传入（HtmlLink 读 href 同机制）。**零侵入实现 ruby 成为可能。**
- 实现：`ruby_word.rs` 新增 `RubyWord` widget（flow Down + 居中：上小字释义 `gloss_label`、下原词 `word_label`），`ScriptHook::on_after_new_scoped` 从 `<ruby t="释义">` 的 `t` 属性读释义；点击发 `RubyWordAction::Clicked{word, gloss}`。
- 标记格式定为 `<ruby t="释义">word</ruby>`（释义走属性，非标准 `<rt>` 子标签——因扩展点把内层文本整体当 widget 文本）。
- Demo：`build_glossed_paragraph()` 生成英文段落（灯塔守夜人，自写文本），10 个生词带中文注音，插为章节第一项。
- 注册链：`makepad_widgets::script_mod` → `crate::ruby_word::script_mod` → app；Html 实例挂 `ruby := mod.widgets.RubyWord{}` 模板。
- 待验证：释义居中显示于词上方、行高撑高但不乱版、点击显示"词 → 释义"、与普通文本/链接混排正常。

## 2026-07-14 — iPhone 17 真机通过，M0 全线收官 ✅✅

- 真机部署完整链路打通：Homebrew Rust 冲突 → DDI 未挂载 → profile 设备不含 UDID → Bundle ID 不一致 → 开发者信任 → 在线验证，全部排完。真机运行流畅、各功能正常（用户确认）。
- 全部坑位整理成 [ios-deploy-guide.md](ios-deploy-guide.md)（9 个坑 + 三方一致原则 + 报错对照表）。
- 下一专题：Ruby 注音（英文单词上方显示中文释义）——本 App 核心特性，先方案讨论后 demo 验证。

## 2026-07-14 — 真机部署排障 + 热重载确认

- `--hot` 热重载用户实测 OK（M0 最后一项补齐）。
- 真机 run-device 失败：cargo-makepad 对 devicectl 失败是 `Err(_)` 吞掉原因再回退 ios-deploy（第三方工具，不支持 iOS 17+，对 iPhone 17 无效）。
- 真实根因：`xcrun devicectl list devices` 显示设备状态 `connected (no DDI)`——开发者磁盘映像未挂载，需 Xcode 对设备完成 "Preparing device for development"。
- 签名后的 .app 产物路径：`target/makepad-apple-app/aarch64-apple-ios/release/readwise-app.app`，可用 `xcrun devicectl device install app --device <ID> <path>` 手动安装看真实报错。
- 命令勘误：列签名资源是 `cargo makepad apple list`（不是 `apple ios list`）。

## 2026-07-14 — M0 收官：iOS 模拟器跑通，闸门通过 ✅

- 修完 Homebrew Rust 冲突后 iOS release 编译 74s 通过；启动模拟器后安装运行成功（iPhone 17 / iOS 26.5）：中文、流式、重渲染均正常（用户截图）。
- 模拟器卡顿：Makepad 纯 Metal 渲染在模拟器走 GPU 软件转译，属已知现象，真机预期流畅；真机验证待用户完成签名三件套。
- 遗留小项：app 图标（resources/icon_1024.png）、窗口标题、`--hot` 热重载专项测试、safe-area 适配（M7）。
- **M0 风险闸门全部通过，MILESTONES.md 已更新。下一步 M1：workspace 重组 + readwise-core 迁移 + App 外壳。**

## 2026-07-14 — M0 第八轮：iOS 构建失败根因——Homebrew Rust 劫持 rustc

- 症状：`rustup target add aarch64-apple-ios-sim` 已装、std rlib 齐全，但 run-sim 仍报 E0463 找不到 core/std。
- 定位：最小复现 + `rustup run stable sh -c 'which rustc'` → 解析到 `/opt/homebrew/bin/rustc`（Homebrew Rust 1.93.1）。机器上有两套 Rust：rustup 1.97.0 与 Homebrew 1.93.1，PATH 里 `/opt/homebrew/bin` 在 `~/.cargo/bin` 前。`rustup run stable cargo` 启动的是 rustup 的 cargo，但它按 PATH 找 `rustc` 时命中 Homebrew 版——它不认识 rustup 的 target 库目录 → E0463。
- 解决：卸载 Homebrew Rust（`brew uses --installed rust` 确认无包依赖它），或 PATH 里把 `~/.cargo/bin` 提到 homebrew 前。
- 教训：桌面构建一直"碰巧"能过是因为 host target 两套都有；交叉编译才暴露混用。

## 2026-07-14 — M0 第七轮：流式通过 ✅，进入 iOS 打包

- 用户实测流式打字机通过。M0 仅剩 iOS。
- 读 tools/cargo_makepad 源码确认 CLI 规则：`--org/--app/--cert/--profile/--device` 必须在 `run-sim`/`run-device` 之前（解析器遇非选项参数即停）；Bundle ID=`{org}.{app}`；iOS target 用 stable channel；`apple ios list` 列签名资源；run-device 缺参会自动打印可用 profile/cert/device。
- 真机免费签名路径：Xcode 建同 Bundle ID 空工程跑一次生成 profile+cert（个人团队 7 天有效）。README 已更新成完整教程。

## 2026-07-14 — M0 第六轮：虚拟化通过 ✅ + 流式打字机验证

- 用户实测：PortalList 200 段滚动流畅 ✓、点词正常 ✓、浅灰链接观感可接受 ✓、重新渲染正常 ✓。**M0 桌面渲染/交互全部通过。**
- 新增流式验证：后台 `std::thread` 每 80ms `Cx::post_action(StreamAction::Chunk)` → `handle_actions` 里 `downcast_ref::<StreamAction>()`（post_action 是 bare action，不能用 as_widget_action）→ Label 逐词上屏。
- 重要发现：makepad 仓库自带 `libs/makepad_ai`（Agent 抽象 + OpenAI 兼容流式后端 + AgentEvent::TextDelta，内部用 SignalToUI），官方 aichat 示例就是流式聊天。**M5 的 AI 链路直接用 makepad-ai，不必自己拼 reqwest+SSE。**
- 待验证：点「流式测试」逐词上屏不卡顿、期间滚动/点词不互相阻塞。

## 2026-07-14 — M0 第五轮：自注册 widget 的引用方式

- 报错 `variable ChapterList not found in scope`：`use mod.widgets.*` 是执行到该行时的快照，看不到同模块后续注册的 widget。官方 todo 的写法：使用处写全路径 `mod.widgets.TodoList{}`（todo/main.rs:234）。已改为 `mod.widgets.ChapterList{}`。

## 2026-07-14 — M0 第四轮：PortalList 虚拟化重构

- 用户反馈：点词 ✓、ruby 降级 ✓，但 3000 链接下滚动粘滞、满屏蓝色链接观感差。
- 重构：参照官方 examples/todo 的模式，自定义 `ChapterList` widget（`#[deref] view: View` + `register_widget`），内部 PortalList 按段虚拟化渲染——`set_item_range` + `next_visible_item` + `list.item(cx, id, id!(Para))` + `set_text` + `draw_all_unscoped`。段落数提到 200 段以证明虚拟化后成本只与可见段落相关。
- 链接样式：`Html{ a.color / a.hover_color / a.pressed_color }` 覆盖为浅灰/悬停蓝。下划线在 `HtmlLink::draw_walk` 里硬编码（`tf.underline.push()` 无条件），M0 接受；正式阅读器不走 `<a>` 包词，M3 做 TextFlow 级命中。
- 数据流：段落存 `static LazyLock<RwLock<Vec<String>>>`（todo 官方同款），draw_walk 读、按钮写。
- 待验证：200 段滚动是否顺滑、点词仍工作、观感改善。

## 2026-07-14 — M0 点词命中验证

- 读源码确认：Html 里 `<a>` 由 `HtmlLink` widget 渲染，点击发 `HtmlLinkAction::Clicked { url }`，且内置 `FingerLongPress`（对移动端长按查词是现成机制）。
- 实现：正文每个词（英文按单词、CJK 按单字）包成 `<a href="w:词">`，`handle_actions` 里 `action.as_widget_action()` + `cast()` 捕获，点词显示在状态栏。约 3000 个 HtmlLink，兼作性能压测。
- ruby 丢字修复：渲染前 `strip_ruby()` 剔除 `<rt>` 保留正文（M4 再做真正的注音渲染）。
- 待验证：点词是否命中、大量链接下滚动/重渲染性能、链接样式是否影响阅读观感。

## 2026-07-14 — M0 桌面渲染验证通过 ✅

- 用户实测（macOS）：章节渲染 ✓、**中文无豆腐字**（makepad 内置字体含 CJK）✓、标题/粗斜体/引用/列表排版 ✓、40 段滚动流畅 ✓、按钮事件计数 ✓（截图确认渲染 13 次）。
- 新发现：`<ruby>` 标签的全部内容被 Html widget **直接丢弃**（非退化为普通文本），"山月/旷野/虎"缺失。结论：进 Html 前必须预处理未知标签（短期：剥掉 `<rt>` 保留正文；M4：自研 ruby 渲染）。
- 小问题：窗口标题是默认 "Makepad"，后续设置。
- M0 剩余：点词命中、iOS 打包、reqwest 流式 → UI。

## 2026-07-14 — M0 第二轮：Rust 侧更新 widget 的正确姿势

- 运行时报 `field chapter not found`：`script_apply_eval!` 按对象字段路径严格解析，不深搜命名实例。官方做法（examples/aichat）是 `self.ui.widget(cx, ids!(chapter)).set_text(cx, &html)` / `self.ui.label(cx, ids!(status)).set_text(...)`——`ids!` 深度查找，`set_text` 对 Html widget 会重新解析 body。已改用此模式。
- 结论：`script_apply_eval!` 适合改样式属性（需完整字段路径），内容更新用 WidgetRef 访问器。

## 2026-07-14 — M0 首次运行：入口协议修正

- 首次 `cargo run` 编译通过，运行时 panic：`AppMain::script_mod not implemented`。dev 分支真实 API 与 makepad-skills 文档有偏差：入口不是 `App::run`，而是 `impl AppMain` 里实现 `fn script_mod(vm) -> ScriptValue`（对照官方 examples/counter 修正）。**教训：以 `~/.cargo/git/checkouts/makepad-*/` 本地源码为准，skills 文档只作参考。**
- 读源码确认：`Html` widget（widgets/src/html.rs）支持 h1-h6、b/i/u/s、code/pre、blockquote、ul/ol、table（含对齐）、a 链接、sub/sup、entities；`body` 属性 + `set_text()` 重新解析。**无 ruby 支持**——M4 注音需自研（方案：扩展 html.rs 的 tag 处理或基于 TextFlow 自定义 widget）。
- 待用户复测：`cargo run -p readwise-app`。

## 2026-07-14 — M0 开工：新项目脚手架

- 新建独立项目 `/Users/ohmycloud/opt/rust/rust-ui/readwise-makepad`（原 Tauri 版 readwise 原样保留），git 已初始化。
- 工程：cargo workspace + `app` crate，依赖 `makepad-widgets` git dev 分支（已确认该分支版本为 2.0.0，Html/Markdown widget 默认启用，无需 feature）。
- M0 demo：窗口 + 头部工具栏 + ScrollYView 内 Html widget 渲染模拟 EPUB 章节（中英混排、标题/引用/列表、`<ruby>` 注音探测、40 段长文滚动），"重新渲染"按钮验证 Rust↔Splash 事件链路（`script_apply_eval!` 更新 body 与状态文本）。
- 技术要点（来自 makepad-skills）：`script_mod!` 冒号语法、`:=` 命名实例、`+:` 合并、`#[source] source: ScriptObjectRef` 必需、含 `e` 的十六进制色需 `#x` 前缀、注册顺序 makepad_widgets 先于 App。
- 已知风险待验证：中文字体（可能豆腐字，需注册 CJK 字体）、ruby 大概率不支持（M4 前需自研 widget）、iOS 打包链路。
- 待用户执行：`cargo run -p readwise-app`，按 README 的 M0 清单逐项反馈。

## 2026-07-14 — 立项

- 决策：放弃 Tauri + React 架构，改用纯 Makepad 2.0（Splash DSL）重写，不引入任何 Web 前端（Vue 方案经确认不采用）。
- 动机：一套 Rust 代码覆盖桌面 + iOS/Android；此前评估 Tauri iOS 适配存在 llamacpp 子进程、sidecar 等多处阻碍。
- 产出：[MILESTONES.md](MILESTONES.md)，M0 为技术验证闸门。
- 学习资料：Makepad 2.0 Book、makepad-skills（14 个 skill，本地已克隆备用）。
- 下一步：等用户确认 milestone 后启动 M0。
