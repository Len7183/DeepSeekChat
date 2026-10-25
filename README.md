# DeepSeekChat

**中文** · [English](README.en.md)

> 本文是主文档，英文对译版见 [README.en.md](README.en.md)（同结构、同事实，改一份请同步另一份）。产品规格 [PRD.md](PRD.md) 目前只有中文。

> **非官方声明**：本项目是由社区开发的 DeepSeek 网页版桌面客户端（Windows），与 DeepSeek 官方无任何隶属关系。DeepSeek 名称与鲸鱼标识归官方所有，此处使用仅用于标识其服务。使用本项目即表示你同意遵守 DeepSeek 官方服务条款。

把 `chat.deepseek.com` 装进一个原生小壳：**托盘常驻、登录不丢、极致省内存**。对话功能 100% 来自官网页面，本项目只做「壳」。

## 下载

| 形态 | 文件 | 说明 |
| --- | --- | --- |
| 绿色单 exe | `DeepSeekChat-v0.1.0-portable.exe`（约 6.7MB） | 放任意可写目录直接用，删除即卸载 |
| NSIS 安装包 | `DeepSeekChat-v0.1.0-setup.exe`（约 3.3MB） | 需要开始菜单/卸载入口时使用 |

- 首次运行 SmartScreen 提示：点「更多信息」→「仍要运行」（无代码签名，仅首次）。
- 数据存 exe 同目录 `data\`；不可写时自动回退 `%APPDATA%\DeepSeekChat\`。
- 检测不到 WebView2 Runtime 时弹窗征得同意后自动在线安装。

## 功能

**窗口**：官网页面 + 系统标题栏，位置/尺寸/最大化记忆，多显示器拔插自动回正，Ctrl+滚轮缩放，图片可直接拖入对话。

**刷新（F5 / Ctrl+R / Ctrl+F5）**：内核级重载，登录与上下文不丢；页面右下角也有刷新按钮。

**全屏（F11）**：随时切换。

**对话导出**：页面右下角工具条 → 5 种格式（Markdown / 纯文本 / 网页 / Word / JSON），文件名 `会话名_日期_时分秒`，保存到 `data\Export session\` 并自动打开目录。导出时自动触发官网打印模式全量挂载消息（虚拟滚动下也能导出完整会话），附件格式化为 `📎 文件名 类型 大小`，思考过程与正文严格分离。

**托盘**：左键显隐切换；右键菜单（打开/设置/退出），菜单跟随官网深浅主题。

**设置**（四项）：开机自启、关闭按钮行为（最小化到托盘/直接退出）、极致省内存模式、数据目录。

**主题跟随**：实测页面背景色，标题栏/边框/设置页/托盘菜单实时同步（含切换过渡色）。

**可靠性**：断网显示本地错误页（原因+重试+20 秒自动重试）；顶层导航白名单（DeepSeek 域内保留，其余交系统浏览器，OAuth 弹窗除外）；渲染进程崩溃 5 秒自动重建；官网「使用环境异常」风控弹窗自动关闭。

**隐私**：零遥测、零上报，数据全部本地；生产日志脱敏；生产版禁用 DevTools。

## 内存（实测）

| 状态 | 预算 | 实测 |
| --- | --- | --- |
| 前台空闲 | ≤180MB | **20MB** |
| 托盘挂起 | ≤80MB | **4MB** |

手段：事件驱动工作集修剪 + WebView2 TrySuspend 真挂起（失败自动降级）+ 隐藏 30 分钟销毁重建。复测：`.\scripts\qa-memory.ps1`。

## 从源码构建

依赖 Windows 10 1809+ x64、Rust stable（MSVC）、Node LTS。

```powershell
.\build-portable.ps1      # 绿色单 exe
.\build-installer.ps1     # NSIS 安装包
```

构建自动重映射本机路径并做隐私扫描（发现本地路径/调试入口/凭据即失败）。开发调试：`settings-ui; npm run dev` + `src-tauri; cargo run`（校准与 QA 入口见 [PRD.md](PRD.md) 附录 C）。

## 目录结构

```
src-tauri/          Rust 外壳（窗口/托盘/内存/主题/导航/导出/WebView2 引导）
settings-ui/        本地页：设置/托盘菜单/错误页（Vite + TS 无框架）
scripts/            图标生成、NSIS 补丁、内存与 UI 的 QA 脚本
build-*.ps1         一键构建
measure-memory.ps1  内存验收
PRD.md              现状规格（每条需求标注实现位置，可对照代码验证）
```

仓库只收源码与配置；运行数据（`data\`）、构建产物（`dist\`）都是本机文件，分发走 Git。

## 已知限制

- 仅 Windows x64；不能多开（WebView2 用户数据目录硬约束）。
- 主题取色与导出选择器依赖官网当前 DOM，大改版后可能需要同步调整（debug 构建 `DSCHAT_EXPORT_PROBE=1` 可校准）。
- 不做代码签名；界面文案仅中文。

## License

MIT —— 见 [LICENSE](LICENSE)。图标素材取自 DeepSeek 官网公开 favicon，商标归官方所有。
