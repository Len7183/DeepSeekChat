# DeepSeekChat 产品需求文档（PRD）

| 项目 | 内容 |
| --- | --- |
| 产品名称 | DeepSeekChat |
| 文档版本 | v0.1.0（未经批准 agent 不得擅自更改或升级软件版本号） |
| 文档性质 | **现状规格**——描述当前代码已经实现的行为，每条需求标注实现位置，可逐条对照验证；不含未实现的规划项 |

---

## 1. 产品概述

### 1.1 一句话定位
Windows 桌面端的 DeepSeek 网页版客户端：把 `chat.deepseek.com` 装进一个常驻托盘、登录长期有效、内存占用极小的原生壳里。

### 1.2 解决的问题
在浏览器里用 DeepSeek 网页版的三个痛点：

1. 没有托盘常驻，容易被误关，重新打开要重新等页面；
2. 浏览器整包内存占用大，且无法按需挂起；
3. 标签页一多就找不到，缺少独立入口与窗口记忆。

本项目**不重新实现对话功能**，全部对话能力 100% 来自官网页面；项目只做「壳」：窗口、托盘、登录态、主题、内存治理、故障兜底、对话导出、构建分发。

### 1.3 明确不做
通知中心（官网不推送）、API 直连客户端、多标签/多账号、插件或主题市场、任何遥测与上报、代码签名证书。

### 1.4 目标用户
需求方本人日常自用 + GitHub 开源用户直接下载预编译产物。

---

## 2. 形态与技术选型

| 决策项 | 结论 | 理由 |
| --- | --- | --- |
| 核心形态 | 官网套壳：主窗口直接加载 `https://chat.deepseek.com` | 功能与官网同步，开发只投在体验层 |
| 技术栈 | Tauri 2.x + Rust 外壳 + 系统 WebView2；本地页 Vite + TypeScript（无框架） | 单文件体积小、内存可控、自带 NSIS 打包 |
| 本地页面 | 随包 3 个：设置页、错误页、托盘菜单（另有 1 个仅 dev server 的控件几何 QA 页 `qa-toggle.html`，不在 Vite 入口、不进产物） | 无框架依赖，构建快 |
| 目标平台 | Windows 10 1809+ / Windows 11，x64 | 唯一支持平台，代码大量使用 Win32 API |
| 协议 | MIT，附非官方与商标免责声明 | 宽松、常用 |

**许可与商标**：DeepSeek 名称与鲸鱼标识归 DeepSeek 官方所有。本项目是社区开发的非官方第三方客户端，与官方无隶属关系，使用标识仅用于指明对应服务。

---

## 3. 运行环境

| 项目 | 要求 |
| --- | --- |
| 操作系统 | Windows 10 1809+ / Windows 11，x64 |
| 运行时 | WebView2 Evergreen（Win11 与多数 Win10 自带）；缺失时由内嵌官方 Bootstrapper 引导安装 |
| GPU | 默认启用硬件渲染；「极致省内存模式」可禁用 |
| 网络 | 需访问 `chat.deepseek.com`；代理跟随系统设置（不自管） |

---

## 4. 功能需求

> 每条均已在 v0.1.0 实现。「实现位置」为源码模块，可直接跳转核对。

### FR-01 主窗口
**实现位置**：`src-tauri/src/actions.rs`、`src-tauri/src/window_state.rs`

- 加载官网，使用系统标题栏（不自绘、不无边框），标题 `DeepSeekChat`；官网 URL 解析失败走错误传播而非 panic（本函数位于托盘点击/二次启动/崩溃自愈等事件路径上）。
- 默认 1200×800，最小 960×600。
- 窗口位置/尺寸记忆到 `data\window-state.json`：外框位置存物理像素、内区尺寸存逻辑像素（DPI 变化后仍正确），并记录缩放系数。
- 最大化状态单独记忆；最大化时保存只更新标记，保留上一次正常态几何，还原后位置正确。
- 多显示器拔插导致坐标越界时自动回退默认位置并居中（与任一显示器有正面积交集即视为可见）。
- 移动/缩放 800ms 防抖保存；失焦时立即补存一次快照。防抖时间戳初值回拨 10 秒（等价「已到期」），用 `checked_sub` 兜住下溢——`Instant - Duration` 在系统运行不足 10 秒时 panic，而本项目 `panic = "abort"`；防抖锁毒化时恢复数据而非放弃（否则位置记忆从此静默失效）。
- 保留 WebView2 默认的 Ctrl+滚轮网页缩放。
- 支持从资源管理器把图片直接拖进对话：必须调用 `disable_drag_drop_handler()`——Windows 上 Tauri 默认替换 WebView2 的拖放处理器、改发 `tauri://drag-drop`，页面收不到 `dataTransfer.files`，官网拖图上传会整个失效。

**验收**：改窗口大小/位置后重启能恢复；拖到副屏后拔掉副屏再启动，窗口仍在主屏内；把图片文件拖到官网输入区，出现附件预览。

### FR-02 单实例
**实现位置**：`src-tauri/src/main.rs`（`tauri_plugin_single_instance` 必须第一个注册）

- 二次启动不新开进程，唤起已运行实例的主窗口。
- 若 webview 已因长时间隐藏被销毁（见 FR-09），二次启动走重建路径而非报错。

**理由**：WebView2 用户数据目录不允许双进程并发使用，属硬约束。

### FR-03 托盘常驻
**实现位置**：`src-tauri/src/tray.rs`

- 托盘图标用应用内置鲸鱼图标，悬停提示 `DeepSeekChat`。
- **左键单击**：主窗口可见且未最小化 → 隐藏到托盘；否则唤起并聚焦。
- **右键**：弹出主题化菜单（打开 / 设置 / 退出）。菜单**不是原生菜单**——原生菜单无法跟随官网深浅主题，用无边框、透明、置顶、不占任务栏的自绘小窗（102×136，3:4）。
- 菜单窗在应用启动 3 秒后后台预创建并隐藏：托盘事件回调内创建 WebView2 控制器会报 `ERROR_INVALID_STATE`；右键早于预建的极端场景由 `ensure_menu_window` 兜底。
- **菜单窗健康检查**：弹出前校验窗口句柄仍可用（`is_visible` 能取到值才算活），失效即销毁重建；销毁失败留 error 日志（残留窗口占着 `menu` 标签会让下次创建因标签冲突失败）。
- 菜单位置按光标所在显示器的缩放换算（`monitor_from_point` 只查一次，避免「尺寸按缩放换算、位置却用光标原值」的混合态），右对齐光标、上方 8px 弹出，贴边钳制在屏内。
- 菜单窗显式清零最小尺寸约束 + 创建后补一次精确设尺：创建期 `WM_GETMINMAXINFO` 早于 tao 约束挂载，系统按默认最小跟踪宽度把 102 强制加宽到 136。
- **收起策略**：低级鼠标钩子（`WH_MOUSE_LL`）监测「点击菜单外」→ 吞掉该次点击并收起。不依赖焦点事件（弹出瞬间主窗口 webview 抢焦点会让菜单闪没），也不要求菜单窗必须拿到焦点。**布防先于 `show()`**：show 与钩子安装之间的窗口期是守卫盲区。钩子安装失败退化为「仅菜单项/Esc 可关」。
- **350ms 收起冷却**：钩子吞掉的菜单外点击若恰落在托盘图标上，托盘 UP 回调仍会送达，立即重开会形成「开→收→开」振荡；冷却静默期内忽略右键弹出（debug 留痕）。

**验收**：右键弹出即点桌面空白处，菜单收起且该次点击不落到桌面；快速连续右键不出现闪烁振荡。

### FR-04 关闭按钮行为
**实现位置**：`src-tauri/src/main.rs`（`WindowEvent::CloseRequested`）、`actions.rs`

- 默认**隐藏到托盘**（不退出），并触发托盘挂起路径。
- 设置可改为**直接退出**。读取配置的锁毒化时恢复真实值（用户的「直接退出」选择不应被默认值覆盖）。
- 最小化按钮始终走任务栏，不拦截、不藏托盘。
- 关闭前无条件保存窗口快照。

### FR-05 设置窗口
**实现位置**：`src-tauri/src/actions.rs`（窗口）、`settings-ui/settings.html`、`settings-ui/src/settings.ts`、`settings-ui/src/settings.css`

独立小窗，522×428，不可缩放/最大化/最小化，居中显示，深浅主题跟随官网。**v0.1.0 性能口径：窗口与 webview 常驻**——首次创建后失焦改为 hide 不销毁，再次打开零等待（对比「失焦 close + 重开建 webview」省 300~600ms）。

**失焦自动隐藏**两条豁免，避免误关：
- 必须曾经拿到过焦点；
- 创建后 1.5 秒内且前台进程仍是本应用 → 判定为主窗口 webview 抢焦点的内部抖动，不隐藏而是重新夺回焦点。

**设置项（全部，共四项）**：

| 设置项 | 控件 | 默认值 | 生效方式 |
| --- | --- | --- | --- |
| 开机自启动 | 开关 | 关 | 立即写注册表 Run 键（带 `--silent-start`） |
| 关闭按钮行为 | 分段：最小化到托盘 / 直接退出 | 最小化到托盘 | 立即生效 |
| 极致省内存模式 | 开关 | 关 | 重启生效（仅文案提示，无弹窗、不自动重启） |
| 数据目录 | 底部小字，单击打开资源管理器 | — | 展示实际生效路径 |

底部另有「关于DeepSeekChat」入口 → 系统浏览器打开项目主页。

**交互细则**（动效只保留在控件本身）：
- 开关：点按乐观更新（即时吸附，不等保存往返）；圆点可拖动跟手，松手过半吸附，快速甩动（>0.4 px/ms）按方向提交；`Enter`/`Space` 键盘切换；拖动时轨道色随进度连续插值；按下时圆点拉伸到 19px、行程收缩为 15px（几何常量在 `settings.css` 与 `settings.ts`（`KNOB_TRAVEL_PRESSED`）两处必须一致）。
- 分段：点按切换；滑块可拖动，松手吸附中心最近的选项；拖动中胶囊宽度在两选项间连续插值（只位移不变宽会让宽胶囊顶到内缘）；位移用不回弹曲线防过冲鼓出圆角框。
- **状态分三层**：已确认（invoke 返回）／期望（乐观更新，界面按它渲染）／待发送（保存进行中产生的新意图）。新意图合并进下一批串行发送；旧响应不覆盖未提交的新意图；保存失败回滚到已确认值并弹 toast，失败之后的新意图保留。
- 手势被系统取消（`pointercancel`）只回弹到期望状态，**不提交**。
- **后端保存语义**（`set_config`，`commands.rs`）：整段持串行锁（并发保存共用同一个 `.tmp` 会互相覆盖），顺序「注册表 → 落盘 → 发布内存」；注册表先读真实状态、一致就不重复写（幂等）；落盘失败回滚注册表，内存态不动；回滚再失败显式报错提示重开设置确认。

**验收**：拖动开关圆点全程跟手不越轨；切换分段胶囊始终不碰圆角框边；点一下主窗口设置窗即关。控件几何可在 `settings-ui/qa-toggle.html` 用真实鼠标拖拽复测（见附录 C）。

### FR-06 数据目录策略
**实现位置**：`src-tauri/src/paths.rs`

- 优先 exe 同目录 `data\`（绿色版/安装版口径一致）；用写探针文件判定可写性。
- 不可写（如装在 `Program Files`）时回退 `%APPDATA%\DeepSeekChat\`，设置窗口底部展示实际路径。
- 目录结构：

```
data\
├─ config.json          设置项
├─ window-state.json    窗口尺寸/位置/最大化
├─ Export session\      对话导出落盘处（FR-22）
├─ webview\             WebView2 用户数据目录（Cookie / 登录态）
└─ logs\                滚动日志
```

- 配置与窗口状态由应用自行读写（原子写：同目录 `.tmp` + rename，Windows 上 rename 走 `MOVEFILE_REPLACE_EXISTING` 本就能覆盖，先 remove 再 rename 反而把写劈成两步、两步之间崩溃会丢旧文件），不使用任何默认存储在 `%APPDATA%` 的 Tauri 插件，保证便携语义一致。

### FR-07 登录持久化
**实现位置**：`src-tauri/src/paths.rs`（`WEBVIEW2_USER_DATA_FOLDER`）

- 在首个 WebView 创建前把环境变量指向 `data\webview\`，登录态随数据目录走。
- 不使用隐身/InPrivate；应用自身任何清理逻辑不得清除 Cookie。
- 隐藏、挂起、销毁重建、重启均不丢登录。

### FR-08 极致省内存模式（禁用 GPU）
**实现位置**：`src-tauri/src/actions.rs`（`browser_args_from_config` / `browser_args_for`）

- 默认**关**（默认 GPU 渲染即须达到 FR-09 指标）。
- 开启后追加 `--disable-gpu --renderer-process-limit=1 --process-per-site`。
- 改动后需重启生效；UI 只在小字里说明，不弹窗、不自动重启。
- **参数在启动时定格**：`main` 里算一次存入 `AppState`，此后 `browser_args_for()` 只返回这份快照。运行期不再随设置变化——否则改完开关后再打开设置窗或销毁重建主窗口，新窗口会带上与既有窗口不同的 Environment 参数。
- **硬约束**：`additional_browser_args` 会整体替换默认参数（须显式保留 Tauri 默认项 `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`）；共享同一用户数据目录的所有 WebView 环境参数必须完全一致——主窗口、设置窗、菜单窗（含重建的主窗口）统一走 `browser_args_for()`。

### FR-09 内存治理与指标
**实现位置**：`src-tauri/src/memory.rs`、`measure-memory.ps1`、`scripts/qa-memory.ps1`

| 状态 | 预算上限 | 口径 |
| --- | --- | --- |
| 前台 | ≤180MB | 主窗口可见聚焦、页面加载完成后无交互空闲 5 分钟 |
| 托盘挂起 | ≤80MB | 窗口隐藏、挂起+修剪后稳定 60 秒 |
| 托盘唤起 | ≤2 秒 | 普通隐藏路径即时恢复；销毁重建路径允许短暂白屏 |

**实测数字不在本文维护**：本文只锁预算上限与口径，数字统一写在 README「内存（实测）」（`README.en.md` 是它的逐节对译副本，两份要同改）。

**验收口径**：`DeepSeekChat.exe` 及**按父进程链归属**的全部 `msedgewebview2.exe` 子进程的**专用工作集（Private Working Set）合计**，排除其他应用的 WebView2 进程；以 `measure-memory.ps1` 输出为准。

**三条手段（默认态即生效，无需开启任何设置项）**：
1. **事件驱动工作集修剪**：对进程树内全部进程执行 `EmptyWorkingSet`；触发时机 = 窗口失焦 / 无交互满 5 分钟 / 隐藏后 / 销毁后。空闲路径**一段空闲只修剪一次**（用户重新活跃后重新布防），失焦、隐藏后、销毁后三条路径不受该标记限制。
2. **WebView2 挂起**：隐藏 1.5 秒后经平台通道调用 `ICoreWebView2::TrySuspend` 真挂起内核。挂起结果以完成回调的 `is_success` 为准——调用报错（官网长连接时的 `ERROR_INVALID_STATE`）与调用成功但内核报告未真正挂起（`is_success=false`）同样降级为纯修剪路径；等待回传用 mpsc + 2 秒超时兜底，超时只结束等待、不取消已排队的调用（调用自身仍会先核对窗口是否隐藏）。
3. **长时挂起销毁**：隐藏超过 30 分钟销毁 webview，唤起时重建（Cookie 在磁盘，登录不丢）。

**用户活跃判定**：注入脚本在 `pointermove/pointerdown/keydown/wheel` 上发心跳，30 秒节流，只表示「活跃」，不含任何内容。

> 注：空闲修剪后专用工作集极低是「按需换页」的正常表现；重新交互后页面按需载回内存，活跃对话时的峰值不计入该指标。

### FR-10 深浅主题跟随
**实现位置**：`src-tauri/src/injected.rs`、`src-tauri/src/theme.rs`、`src-tauri/src/main.rs`（事件入口）

两段式（对标 `deepseek-ai/deepseek-harness` 的启动回退色 + 页面实测色）：
1. 启动回退色：深色 `#1b1b1c` / 浅色 `#f9fafb`，标题栏符号色对应 `#f9fafb` / `#0f1115`；
2. 注入脚本实测页面背景色后经 `dschat://theme` 事件上报，立即替换。

- **事件入口白名单**：`bg` 字段仅接受 ASCII 的 `#RRGGBB`（长度 7、`#` 开头、其余全为十六进制数字）。任意字符串透传会让下游 `theme::parse_hex` 的按字节切片在 6 字节非 ASCII 输入（如 `"日日"`）上 panic——`panic = "abort"` 下远程页面可借此杀死进程。
- 颜色解析兼容两种序列化：旧式逗号 `rgb(255, 255, 255)` 与 CSS Color 4 空格式 `rgb(255 0 0)` / `rgb(0 0 0 / 0.5)`（Chromium ~M121 起 `getComputedStyle` 返回空格式，不兼容则主题检测整体失效）。
- 取色候选链：`html → body → 前 5 层应用容器`，取第一个不透明背景色；全透明时兜底扫描页面元素取最高频背景色（strict 模式不回退全扫）。
- 深浅判定：相对亮度 < 0.55 记为深色。
- 跟随速度：DOM 的 class/style 一变即同帧测量上报（MutationObserver 不筛属性类型 + subtree 全域），并启动 `requestAnimationFrame` 逐帧跟踪跟住官网 CSS 过渡色（单次跟踪续 1 秒、上限 3 秒）；颜色未变不产生 IPC（同 key 去重，去重同时联动页面内 `dschat-theme` CustomEvent 广播，导出工具条同帧变色）。
- **1s 轮询兜底**：strict 廉价探测（只测候选链、不触发全扫），任何观察不到的主题变化路径最迟 1 秒跟上；全扫机会留给 DOM 事件路径。
- 同步范围：主窗口与设置窗口的标题栏、文字、边框色；设置页与托盘菜单的深浅样式（`theme-changed` 事件广播）。
- 边框色由背景色派生：浅色加深 15%、深色提亮 12%。
- 平台差异：Windows 11 用 DWM `CAPTION_COLOR/TEXT_COLOR/BORDER_COLOR` 精确着色；Windows 10 仅支持沉浸式深浅切换，精确属性调用失败即忽略。

### FR-11 WebView2 运行时引导
**实现位置**：`src-tauri/src/webview2_boot.rs`

- 启动时按微软官方口径检测（`EdgeUpdate\Clients\{F3017226-...}` 的 `pv` 值，覆盖 HKLM/WOW6432Node/HKCU 三处）。
- 缺失且在线：原生对话框征得同意 → 释放内嵌的微软官方 Bootstrapper 到 `%TEMP%`，`/silent /install` 等待完成 → 复检通过则继续启动。
- 缺失且离线：对话框给出微软官方下载页与手动指引，并打开该页。
- 用户拒绝或安装失败：给出说明后退出进程。
- 本模块跑在 logger 安装之前，过程由对话框当面传达，「曾引导安装过」这一事实经返回值在 setup 里补记日志（见附录 B 第 6 条）。
- 安装包版由 NSIS 流程负责引导（Tauri `downloadBootstrapper` 模式）。
- 另提供 `alert_box`：webview 不可用或不可信路径上的原生提示（启动失败、主窗口重建失败等用户可感知反馈）。

### FR-12 断网 / 加载失败兜底
**实现位置**：`src-tauri/src/webview_cfg.rs`、`settings-ui/error.html`、`settings-ui/src/error.ts`

- 监听 `NavigationCompleted`，失败且当前不在错误页时跳转本地错误页（防循环）。
- 监听注册归在主窗口创建流程内（`create_main` → `attach_failure_handler`）：监听绑定在当前 WebView2 实例上，销毁重建的主窗口同样被监听。**注册失败必须醒目留 error 日志**——静默失效等于错误页整条链路消失。
- **错误页判定用精确 URL 比较**（解析后比对 host+path）：子串匹配会把带 `/error.html` 查询串/远端路径误判为错误页而漏兜底。
- **TOCTOU 重校验**：事件回调内不直接导航，延迟 120ms 后重读当前 URL——失败潮（一次断网产生多批 NavigationCompleted）或已落在错误页时不重复跳转。
- 错误页展示：具体原因（WebView2 的 `WEB_ERROR_STATUS` 映射为中文）+「重试」按钮 + 网络/代理检查提示 + 20 秒自动重试倒计时。
- 手动点「重试」立即停掉自动倒计时；重试失败恢复按钮并提示。错误页 F5 与重试按钮同路径。
- 错误码经 URL fragment 传入；本地页地址 debug 走 Vite dev server、release 走内嵌资源。
- WebView2 初始化失败同样落在本地错误页路径。

### FR-13 导航、外链与登录弹窗
**实现位置**：`src-tauri/src/webview_cfg.rs`

- 顶层导航白名单：`deepseek.com` 及其子域 + 应用本地页（`tauri.localhost` 恒放行；`localhost`/`127.0.0.1` 仅 debug 构建——release 放行任意本地端口会让页面重定向到本机其他服务时留在应用内）。
- 其余域名一律交给系统默认浏览器，导航返回 false；日志只记域名。
- `file:` 协议直接拦掉、**不**外抛：把文件拖到页面非投放区时 WebView2 原生默认行为是导航到 `file://`，若照旧外抛会用系统默认程序打开该文件（拖一下图片就弹出看图器）。
- **弹窗与新窗口白名单制**：DeepSeek 域 + 主流 OAuth 提供商（`login.microsoftonline.com` / `accounts.google.com` / `github.com` / `login.weixin.qq.com` / `open.weixin.qq.com` / `graph.qq.com`）→ `NewWindowResponse::Allow`（应用内打开，与主窗口同浏览器进程、共享 Cookie，登录回调可完成）；其余域名 `Deny` 并交系统浏览器。**无条件 Allow 等于给 `target=_blank` 开了绕过 `on_navigation` 白名单的旁路**（Allow 弹窗的内部导航不经过 on_navigation）。

### FR-14 滚动日志
**实现位置**：`src-tauri/src/logging.rs`

- 写入 `data\logs\deepseekchat*.log`，单文件上限 1MB，启动时清理为最近 4 个（磁盘约 5MB 上限）。
- 时间戳用**本地时区**（`TimezoneStrategy::UseLocal`）；插件默认 UTC 与操作时间差整小时，排障对不上。注意它会连带覆盖默认 format（行格式变为 `[日期][时间][级别][target] 消息`）。
- 记录：启动、数据目录初始化结果、窗口创建/唤起/隐藏、关闭行为、挂起/修剪/销毁、主题上报、导航失败与外链、配置保存、托盘菜单弹出与收起、导出结果、WebView2 引导安装（检测发生在 logger 安装之前，只记「曾由引导程序安装」这一结果）。
- **脱敏**：生产版不主动记录 Cookie、请求头、带 token 的完整 URL、本地绝对路径或任意网页载荷；外链与主页只记动作或域名。任意页面日志回传只在 debug 构建启用。公开日志前仍需检查第三方错误文本中的账号、路径与凭据。
- 生产构建 Info 级别、禁用 DevTools，日志是首要排障手段；debug 构建 Debug 级别并额外输出到 stdout。

### FR-15 安装包（NSIS）
**实现位置**：`build-installer.ps1`、`src-tauri/nsis/installer.nsi`、`scripts/patch-nsis-template.mjs`、`tauri.conf.json`

基于 Tauri 官方模板（`tauri-cli-v2.12.0`）打补丁，构建脚本在模板缺失时自动下载并重新打补丁：

- 安装模式二选一：全局安装（`Program Files`，需管理员）/ 当前用户安装（免管理员），均可改安装路径。
- 语言：简体中文 + English，且 `displayLanguageSelector: true`。必须开启选择器：关闭时 NSIS 不会做系统语言自适应，英文系统同样看到中文向导。
- Finish 页两枚勾选：桌面快捷方式（默认创建，复用 MUI 的 SHOWREADME 位）、开始菜单快捷方式（默认不创建）。开始菜单那枚是自建控件，**必须在页面 SHOW 回调里用 nsDialogs 创建**（此时才挂进同一个页面对话框、共享 DPI 缩放；用裸 `CreateWindowEx` 按像素定位会整枚落到向导图下面）。开机自启**不在安装向导里**，只在应用设置中提供。
- 卸载：默认删除用户数据（`$INSTDIR\data` 与 `%APPDATA%\DeepSeekChat` 中实际存在者），勾选「保留用户数据」则跳过。
- 安装/卸载向导图标：应用鲸鱼图标。
- 体积门禁：>8MB 构建失败。

### FR-16 绿色单 exe
**实现位置**：`build-portable.ps1`

- 单文件 `DeepSeekChat.exe`，内嵌全部前端资源与 WebView2 Bootstrapper（缺失时自动从微软官方下载）。
- 必须启用 `tauri/custom-protocol` feature，否则本地页仍路由到 devUrl、发布产物全白屏。
- 双构建脚本使用相同的 `--remap-path-prefix`（仓库、用户目录、Cargo/Rustup 目录 → `/build`）；编译后运行 `scripts/check-release.mjs` 检查未压缩主程序：内嵌本地绝对路径、调试入口标记或已知凭据签名时阻止发布。
- 体积门禁：>15MB 构建失败。
- 卸载 = 删除 exe 与 `data\` 目录。开启过自启后手动删除程序，注册表 Run 键会残留指向不存在路径的键值（无实际影响），README 给出手动清理方法。

### FR-17 图标与品牌
**实现位置**：`scripts/gen-icon.mjs`、`src-tauri/icons/`

- 素材取自 DeepSeek 官网公开 favicon；脚本解析 ICO 内最大条目（支持 PNG 条目与 32bpp BMP 条目，后者手动解码）→ 圆角蒙版（22%）→ 输出 256/128/64/48/32/16 PNG，并把这 6 张 PNG 直接封装成 `icon.ico`（PNG 压缩条目，约 31KB）。不引入把 PNG 解码成 BMP 条目的依赖（那样条目膨胀到约 370KB）。
- 同一 `icon.ico` 同时用于：exe 内嵌图标、托盘图标、NSIS 安装/卸载向导图标。
- 仓库注明素材来源与商标归属。

### FR-18 官网风控弹窗自动关闭
**实现位置**：`src-tauri/src/injected.rs`

- 官网对全新浏览器环境会弹「使用环境异常」提醒。注入脚本在页面加载后 90 秒内轮询（递增间隔 500ms/1s/1.5s）+ DOM 变化即扫（限频 1 次/秒），定位弹窗标题 → 点击同弹窗内的图标关闭钮；找不到关闭钮则移除整个弹窗挂载点。
- 成功后写日志「已自动关闭官网环境提醒弹窗」。
- 登录后会话被记住，该弹窗通常只出现一次。

### FR-19 开机自启与静默启动
**实现位置**：`src-tauri/src/main.rs`、`tauri_plugin_autostart`

- 设置中开启，写入 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，参数带 `--silent-start`；写前先读注册表真实状态，与目标一致不重复写。
- 带该参数启动时不显示主窗口，静默进入托盘；其余初始化（数据目录、托盘、内存监视）照常。

### FR-20 远程页与本地页的通信边界
**实现位置**：`src-tauri/capabilities/default.json`、`src-tauri/src/main.rs`

- 授权窗口：`main`、`settings`、`menu`、`oauth-*`；远程源限定 `https://chat.deepseek.com/*` 与 `https://*.deepseek.com/*`。
- 远程页（官网）**不能直接调用应用命令**（ACL 限制），注入脚本一律通过 `plugin:event|emit` 上报 `dschat://theme` / `dschat://heartbeat` / `dschat://log` / `dschat://reload`（FR-21）/ `dschat://export`（FR-22）/ `dschat://fullscreen`（FR-23），由 Rust 侧 `listen_any` 接收。
- 事件入口的载荷校验：`dschat://theme` 的 `bg` 仅收 ASCII `#RRGGBB`（防 panic，见 FR-10）；`dschat://export` 空载荷拒绝落盘（见 FR-22）；页面日志回传仅 debug 落盘。
- 本地页（设置/菜单/错误）可直接 `invoke` 应用命令。
- 应用命令清单：`get_config` / `set_config` / `get_data_dir` / `open_data_dir` / `open_project_home` / `menu_action` / `retry_navigation` / `get_theme`（退出走 `menu_action("quit")`，不另设命令）。
- `menu_action` / `retry_navigation` 声明为 async 命令：避免在主线程 webview 回调内执行窗口操作/创建 WebView（`ERROR_INVALID_STATE`）。

### FR-21 刷新
**实现位置**：`src-tauri/src/injected.rs`（快捷键拦截）、`src-tauri/src/actions.rs::reload_main`、`settings-ui/src/error.ts`（错误页 F5）

- 快捷键：**F5 / Ctrl+R / Ctrl+F5** 在官网页内触发主窗口刷新。wry 默认关闭 WebView2 浏览器加速键（`AreBrowserAcceleratorKeysEnabled=false`），按键会落到页面，注入脚本捕获后经 `dschat://reload` 事件请求壳执行。
- 刷新走 `ICoreWebView2::Reload()`：内核级重载，保留 SPA 会话语义、不产生历史条目。Ctrl+F5 与普通刷新同路径——WebView2 Reload 无绕缓存语义，官网静态资源自带版本化 URL，无需强刷。
- webview 已因长挂起被销毁时，刷新降级为重建主窗口。
- 错误页（本地页，无注入脚本）F5 触发 `retry_navigation`，与重试按钮同路径。
- 托盘菜单不提供刷新项：刷新是页面级操作，保持菜单三项克制。

### FR-22 对话导出
**实现位置**：`src-tauri/src/export.rs`（页面脚本 + 落盘）、`src-tauri/src/actions.rs`（注入）、`src-tauri/src/main.rs`（事件监听）

- 参考对象：Edge 商店扩展 Chat2File（对话导出工具）。**不引入扩展运行时**，取其核心路径——「识别对话 DOM → 转换 → 保存」——在注入脚本内实现，零新增依赖。
- 入口：官网页右下角悬浮工具条（刷新钮 + 导出钮）。**登录页与新会话欢迎页（无 `.ds-message`）工具条自动隐藏**，出现对话后 120ms 去抖内带 260ms 弹性动画滑入。
- 抓取（2026-10 实测官网 DOM，见探针 `DSCHAT_EXPORT_PROBE=1`）：轮单元 = `.ds-message`；用户轮 = 不含 `.ds-markdown.ds-assistant-message-main-content` 的消息节点（纯文本气泡）；助手轮正文只认 `.ds-assistant-message-main-content` 标记（思考容器内也存在 `.ds-markdown`，宽匹配会把思考误当正文），思考块 `.ds-think-content` 单独成节且排除自包含。
- **虚拟列表全量挂载（完整性核心）**：官网消息区是 virtua 虚拟列表（`ds-virtual-list--printable`），DOM 只挂视口附近的消息，且**不是原生滚动**（`scrollHeight` 恒定、`scrollTop` 恒 0，内部用 transform 移动）——逐屏滚动方案实测无效（同一会话导出轮数时多时少）。`--printable` 是 virtua 的打印模式标记：**派发 `beforeprint` 事件会全量挂载所有条目，`afterprint` 恢复虚拟化**——这正是导出需要的官方通道。流程：`beforeprint` → 等挂载稳定（连续 5 帧计数不变 ≈80ms，180 帧兜底）→ 收集 → `afterprint`。
- **智能取文本**：附件气泡里「文件名/类型标签/大小」是相邻的独立子元素（如 `DIV.eafda4ae`），裸 `textContent` 会挤成 `xxx.txtTXT 56.01KB`——`nodeText` 遍历子树，块级元素（DIV/P/LI/TABLE/TR/H1-6/BLOCKQUOTE/PRE/BR）边界补换行、行内元素边界补空格，过滤 SCRIPT/STYLE/BUTTON，再压缩多余空白。
- **附件格式化**：克隆消息节点并移除附件容器（`.eafda4ae, [class*="file-card"], [class*="attachment"]`）后取正文（附件文本不混入），附件行从叶子容器文本单独提取；一个容器装多个附件时用**正则字面量**拆「文件名 类型 大小」三元组（类型标签须为大写独立词、正则无 `i` 标志，文件名里的小写 `.txt` 不会误配），每个附件独立一行 `📎 文件名 类型 大小`。克隆-移除法彻底避免正文与 📎 行内容重复。
- 格式（5 种，文件名统一「会话名_YYYY-MM-DD_HH-MM-SS」，标题限 24 字符防资源管理器截断时间部分）：

  | 格式 | 生成方式 |
  | --- | --- |
  | `.md` Markdown | 页面侧手写 HTML→Markdown（代码块带语言/表格/列表/引用/链接），思考块为引用节 |
  | `.txt` 纯文本 | 页面侧 `nodeText` 分节（与 Markdown 版各自独立收集，附件格式对两版一致生效） |
  | `.html` 网页 | 页面侧生成独立样式单文件 |
  | `.doc` Word | Word 可直接打开的 HTML 单文件（**原位注入** Office 命名空间与 WordDocument 声明，不重组标签——重拼会把 `<title>` 挤到 `</head>` 之后），不引入 docx 生成器 |
  | `.json` 数据 | 结构化 `{user, thinking, assistant}` 数组，`version: "0.1.0"` |

- 落盘：`dschat://export` 事件（FR-20 通路）→ `data\Export session\`；落盘后自动打开导出目录。
- **空载荷防护**：内容字段缺失/非字符串/纯空白时不落盘（空文件以成功姿态打开目录是对用户的欺骗），失败只记日志。
- **文件名唯一化**：时间戳 `YYYY-MM-DD_HH-MM-SS`（`GetLocalTime` 本地时间）+ 进程内导出序号后缀（消除同秒 check-then-act 覆盖竞态：两次导出在同一秒时 `exists()` 检查与实际写入之间存在窗口，后写覆盖先写）+ 重名追加 `(n)`。
- **无 UI 反馈**（用户指定）：emit 的 Promise 拒绝静默吞掉，无 toast/横幅；成功路径由「自动打开导出目录」天然反馈，失败由日志承载。
- 边界：单次导出上限 16MB；日志只记文件名不记路径（FR-14）；上报载荷仅纯文本，不含 URL / Cookie / 输入框内容。
- UI 主题跟随：订阅标题栏同源信号（`dschat-theme` CustomEvent，同帧变色）+ `pendingDark` 挂载期补应用 + 1s 轮询自测兜底（事件源 60 秒内活跃时轮询让位，事件源死亡后自测恢复，避免陈旧缓冲值把主题永久钉死）；`setDark` 无条件 toggle（若「值未变就返回」，刷新场景事件早于 body 就绪时 mount 后永远补不上 class）。

**验收**：长会话（60+ 轮）导出 md/txt/json 轮数一致且完整；带附件会话导出后每附件一行 `📎 文件名 类型 大小` 且与正文不重复；连续两次快速导出生成两个文件。

### FR-23 全屏
**实现位置**：`src-tauri/src/injected.rs`（F11 拦截）、`src-tauri/src/webview_cfg.rs::toggle_fullscreen`

- `F11` 在官网页内切换主窗口全屏（tauri `set_fullscreen` 翻转）；注入脚本拦截后经 `dschat://fullscreen` 事件请求壳执行，与刷新同一通道模式。
- 不设菜单项与设置项：系统级通用快捷键，再按 F11 退出。

### FR-24 渲染进程崩溃自愈
**实现位置**：`src-tauri/src/webview_cfg.rs`（`attach_failure_handler` 内注册 `add_ProcessFailed`）

- 实测场景：WebView2 渲染进程偶发 `STATUS_ACCESS_VIOLATION` 崩溃（内核/驱动行为，壳无法阻止），页面白屏。
- 自愈路径：监听 `ProcessFailed`，kind 为渲染退出（1）或渲染无响应（2）时延迟 5 秒销毁残留窗口并重建主窗口；Cookie 在磁盘，登录态不丢，会话由官网 SPA 自行恢复。
- **单飞守卫**：一次崩溃产生多批事件（渲染退出→无响应→帧失败），`compare_exchange` 原子抢锁保证只有一个重建线程在跑，恢复完成后复位；重复事件只记 debug 日志。
- 重建失败弹原生 `alert_box` 提示重启（仅记日志会让用户表现为「白屏后没反应」）。
- 自愈重建走 `actions::create_main`（导航失败监听与 ProcessFailed 监听随创建流程重新注册，与 FR-12 同源设计）；监听注册失败留 error 日志。

---

## 5. 非功能需求

| 类别 | 要求 | 现状 |
| --- | --- | --- |
| 体积 | 绿色 exe ≤15MB，安装包 ≤8MB（构建脚本硬门禁） | 达标 |
| 启动 | 冷启动 ≤2s（双击到主窗口可见，首建用户数据目录不计入） | 达标 |
| 隐私 | 零遥测、零上报、数据全本地、日志脱敏、代理跟随系统；发布产物重映射构建机路径并经隐私扫描 | 满足 |
| 兼容性 | Win10 1809+ / Win11 x64；WebView2 Evergreen 不锁版本；DPI PerMonitorV2 | 满足 |
| 可靠性 | 断网兜底、渲染崩溃自愈、异常退出后不丢登录与设置、窗口坐标越界自动回退、配置文件损坏回退默认值、锁毒化恢复而非放弃 | 满足 |
| 安全 | 生产版禁用 DevTools；导航/弹窗双白名单；`file:` 拦截；IPC 事件入口载荷校验（bg 白名单防 panic）；CSP 未设（外壳不注入自有内容，注入脚本仅主题/心跳/风控/快捷键/导出） | 已知取舍 |

---

## 6. 交付物

1. 完整源码（GitHub 仓库，MIT）。
2. `build-portable.ps1` / `build-installer.ps1` —— 一键构建双形态，含工具链检查、素材自动下载、构建机路径重映射与发布隐私检查（`scripts/check-release.mjs`）、体积门禁。
3. `measure-memory.ps1`（单次/监控两种模式）+ `scripts/qa-memory.ps1`（完整验收序列）。
4. GitHub Actions：打 `v*` tag 触发，windows-latest 构建双工件并发布 Release；安装包步骤以 `-SkipFrontend` 复用绿色版步骤的前端产物，不重复 npm install / tsc / vite build。
5. README 中英双份：`README.md`（中文主文档）+ `README.en.md`（英文对译，逐节对应，改一处需同步另一处）。
6. `scripts/` 下的被动 UI QA 工具（窗口截图、窗口枚举、失焦实测、点击注入）；控件几何另见浏览器 QA 页 `settings-ui/qa-toggle.html`。

---

## 7. 已知取舍与风险

| 项 | 说明 |
| --- | --- |
| 不代码签名 | 自用 + 开源，不购证书；首次运行可能触发 SmartScreen，README 给出「更多信息 → 仍要运行」的处理 |
| `TrySuspend` 常失败 | 官网长连接导致 WebView2 拒绝挂起（`ERROR_INVALID_STATE`），实测纯修剪路径即可达标，故接受降级 |
| 内存指标依赖空闲口径 | 空闲修剪后工作集极低是按需换页；活跃对话峰值不计入该指标 |
| 设置窗失焦即隐藏 | 设计如此（对标 macOS 面板），代价是无法在设置窗与主窗口间并存操作 |
| 销毁重建路径 | 隐藏超 30 分钟后唤起允许短暂白屏 |
| Bundle identifier 以 `.app` 结尾 | `tauri-cli` 构建时告警；本项目仅发 Windows，无实际影响 |
| 依赖官网 DOM 结构 | 导出抓取、主题取色、风控弹窗关闭基于 2026-10 页面结构，官网改版可能需跑探针重新校准（`DSCHAT_EXPORT_PROBE=1`） |
| 单实例为硬约束 | 无法多开不同账号；二次启动只会唤起已有实例 |
| 本地页文案只有中文 | 设置页/托盘菜单/错误页未做 i18n；安装向导双语（需语言选择器开启，见 FR-15） |

---

## 附录 A：决策记录

| # | 议题 | 结论 |
| --- | --- | --- |
| 1 | 产品形态 | 官网套壳，非 API 客户端 |
| 2 | 技术栈 | Tauri 2；本地页 Vite + TS 原生，无框架 |
| 3 | 内存指标 | 前台 ≤180MB / 托盘 ≤80MB，口径=专用工作集合计 |
| 4 | 省内存模式 | 默认关；默认态靠修剪+挂起达标 |
| 5 | 标题栏 | 对标 deepseek-harness 两段式取色 |
| 6 | 托盘行为 | 手动启动显示主窗、自启静默进托盘、关闭→托盘、左键切换、右键自绘菜单 |
| 7 | 数据目录 | 跟随 exe，不可写回退 `%APPDATA%` 并在设置展示 |
| 8 | WebView2 引导 | 绿色版内嵌 Bootstrapper；安装版 NSIS 引导 |
| 9 | 外链策略 | 顶层导航白名单 + 弹窗白名单（DeepSeek 域 + OAuth 提供商），`file:` 拦截不外抛 |
| 10 | 挂起恢复 | 接受销毁重建的短暂白屏，换 ≤80MB 硬指标 |
| 11 | 设置项 | 仅四项：自启 / 关闭行为 / 省内存 / 数据目录（另有项目主页入口） |
| 12 | 安装包 | NSIS 全局/当前用户二选一，可改路径，Finish 页两枚勾选；自启只在应用设置里 |
| 13 | 通知 | 不做（官网不推送） |
| 14 | 设置窗生命周期 | 失焦自动隐藏（常驻不销毁，重开零等待） |
| 15 | 协议/版本 | MIT；v0.1.0 |
| 16 | 配置保存 | 串行锁 +「注册表 → 落盘 → 发布内存」；失败回滚注册表、内存态不动 |
| 17 | 省内存模式生效时机 | 浏览器参数启动时定格、重启才生效（WebView2 Environment 参数一致性硬约束） |
| 18 | 发布隐私 | 路径重映射 `/build`；`check-release.mjs` 扫描绝对路径/调试入口/凭据签名，命中即阻止发布 |
| 19 | 对话导出形态 | 内置注入脚本（参考 Chat2File 核心路径），不引入扩展运行时；零新增依赖 |
| 20 | 刷新语义 | F5/Ctrl+R/Ctrl+F5 同走 `ICoreWebView2::Reload`；无独立强刷 |
| 21 | 导出选择器来源 | 不押注猜测：debug 探针实测官网 DOM 后定稿；改版重跑探针校准 |
| 22 | 导出完整性 | virtua 虚拟列表非原生滚动、滚动方案实测无效 → 走 `beforeprint` 打印模式全量挂载（官方通道） |
| 23 | 导出文件命名 | 统一「会话名_YYYY-MM-DD_HH-MM-SS」+ 同秒进程序号 + 重名 `(n)`；标题限 24 字符 |
| 24 | 附件导出格式 | 克隆-移除附件容器取正文 + 正则字面量拆三元组 → `📎 文件名 类型 大小` 每附件一行 |
| 25 | 导出无反馈 | 用户指定：无 toast/横幅，成功以自动打开目录为反馈，失败走日志 |
| 26 | 启动/加载性能 | 托盘菜单窗延迟 3s 后台预建；设置窗常驻隐藏；风控弹窗扫描递增间隔 + 观察器限频；PDF/PNG 导出因依赖重、链路长而移除（保留 md/txt/html/doc/json 5 种） |

---

## 附录 B：模块地图

```
src-tauri/src/
├─ main.rs          启动顺序编排、6 个 dschat:// 事件监听、命令注册、窗口事件路由
├─ paths.rs         数据目录解析与可写性回退（最先执行）
├─ webview2_boot.rs WebView2 Runtime 检测与引导（在任何 WebView 之前）、alert_box
├─ config.rs        设置项读写（原子写）
├─ window_state.rs  窗口几何记忆与越界校验
├─ actions.rs       主/设置窗口创建、唤起、隐藏、刷新、浏览器参数快照
├─ tray.rs          托盘图标、自绘菜单窗、低级鼠标钩子收起守卫、350ms 冷却
├─ memory.rs        工作集修剪、TrySuspend、长挂起销毁、空闲监视
├─ theme.rs         主题状态、DWM 标题栏/边框着色、事件广播
├─ injected.rs      注入官网页脚本（主题实测/心跳/风控弹窗/刷新与全屏快捷键）
├─ export.rs        对话导出：悬浮工具条 + beforeprint 全量挂载抓取 + 5 种格式落盘
├─ webview_cfg.rs   导航与弹窗白名单、错误页兜底、崩溃自愈、全屏
├─ commands.rs      IPC 命令
└─ logging.rs       滚动日志与旧日志清理

settings-ui/
├─ settings.html + src/settings.{ts,css}   设置页
├─ menu.html     + src/menu.{ts,css}       托盘菜单页
├─ error.html    + src/error.{ts,css}      错误页
├─ qa-toggle.html                           控件几何 QA 台（仅 dev server，不在 Vite 入口）
└─ src/theme.css                            三页共用的深浅色变量
```

**七条改代码前必须知道的不变量**：

1. 所有 WebView 窗口（main / settings / menu）的 `additional_browser_args` 必须完全一致——共享 WebView2 用户数据目录的硬约束，统一走 `actions::browser_args_for()`；该函数返回启动时定格的快照（`browser_args_from_config()` 只在启动算一次），省内存模式重启才生效。
2. 远程页只能经 `plugin:event|emit` 上报，不能直接 `invoke` 应用命令（ACL）；事件入口必须校验载荷（`bg` 白名单、export 空载荷拒收）——`panic = "abort"` 下任何会 panic 的输入都是远程页的进程杀死开关。
3. 不得在托盘事件回调或同步命令里创建 WebView 窗口（`ERROR_INVALID_STATE`）——菜单窗预创建，`menu_action` / `retry_navigation` 声明为 async 命令。
4. 不使用任何默认存储在 `%APPDATA%` 的插件，配置与窗口状态自写 `data\`，否则破坏便携语义。
5. 三个本地页（`settings.html` / `menu.html` / `error.html`）的 `<head>` 必须保留 `<link rel="icon" href="data:,">`：不声明时 WebView2 会请求 `/favicon.ico`，资源协议里没有这个文件，`tauri::manager` 每次打开本地页都记一条 ERROR 噪音。
6. `paths::init()`、`webview2_boot::ensure_runtime()` 跑在 `tauri::Builder` build 之前，此时 logger 尚未安装，二者内部的 `log::*` 一律不落盘。需要留痕的启动期事实在 `setup` 里补记，或在模块间用返回值传递。
7. **注入 JS 内的正则必须写字面量 `/…/g，禁止 `new RegExp(string)`**——Rust 原始字符串里的 `\\S` 经 JS 解析再进 RegExp 构造器会被吃掉一层转义（实测 `.source` 变成 `(S+)`），且此类错误不报错、只让匹配静默失效；正则字面量无此问题。同理，类型标签匹配不得加 `i` 标志（防文件名里的小写扩展名误配）。

---

## 附录 C：调试与 QA 入口

| 入口 | 作用 | 生效构建 |
| --- | --- | --- |
| `--silent-start` | 静默进托盘（开机自启路径） | 全部 |
| `--settings` | 启动即打开设置窗口 | debug |
| `--menu-test` | 在固定屏幕位置显示托盘菜单窗 | debug |
| `--error-test` | 主窗口直接导航到本地错误页 | debug |
| `--ext-test` | 导航到外部域名，验证外链策略 | debug |
| `DSCHAT_CMD_TEST=1` | 8 秒后从菜单窗与主窗口分别探测命令可达性，结果回传日志 | debug |
| `DSCHAT_PROBE=1` | 注入诊断横幅（IPC 可用性 / 注入标记 / 实测背景色） | debug |
| `DSCHAT_DIALOG_PROBE=1` | 探测官网风控弹窗 DOM 结构（10/20/35 秒三批） | debug |
| `DSCHAT_EXPORT_PROBE=1` | 探测官网对话 DOM 结构（FREQ/MDCHAIN/SIDE/KIDS/MSGS/ATTACH 六条回传），FR-22 抓取校准 | debug |
| `DSCHAT_EXPORT_TEST=<格式名>` | 18 秒后自动点开侧栏首个会话并触发一次指定格式导出（菜单项文字匹配） | debug |
| `measure-memory.ps1 -Mode foreground\|tray`（另可加 `-Monitor <秒>`） | 内存指标测量 | — |
| `scripts/qa-memory.ps1` | 完整验收序列（空闲 5.5 分钟 → 隐藏挂起 → 唤起计时） | — |
| `scripts/qa-windows.ps1` | 被动 UI QA：PrintWindow 截图、失焦实测、前台归属观察 | — |
| `scripts/capture-window.ps1` / `click-at.ps1` / `diag-windows.ps1` | 单窗口截图 / 坐标点击 / 窗口枚举诊断 | — |
| `settings-ui/qa-toggle.html` | 设置页开关与分段胶囊的浏览器几何 QA 台（`cd settings-ui; npm run dev` 后开 `http://localhost:5173/qa-toggle.html`），自带 `invoke` 桩不依赖 Rust；不是 Vite 入口、不进产物 | 浏览器 |

> 标注 debug 的入口只在 debug 构建参与编译（`#[cfg(debug_assertions)]`），release 产物不响应；`scripts/check-release.mjs` 会扫描发布主程序中的调试入口标记，出现即阻止发布。
