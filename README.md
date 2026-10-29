# DeepSeekChat

**中文** · [English](README.en.md)
> 本文是主文档，英文对译版见 [README.en.md](README.en.md)（同结构、同事实，改一份请同步另一份）。

> **"非官方"声明**：本项目是由社区开发的 DeepSeek 网页版桌面客户端（Windows），与 DeepSeek 官方无任何隶属关系。DeepSeek 名称与鲸鱼标识归官方所有，此处使用仅用于标识其服务。使用本项目即表示你同意遵守 DeepSeek 官方服务条款。

## 项目解决了什么问题

在浏览器里用 DeepSeek 网页版的三个痛点：

1. **容易误关**——没有托盘常驻，手一滑就关掉标签页，重开要重新等页面加载；
2. **内存占用大**——整包浏览器常驻后台，无法按需挂起；
3. **缺少独立入口**——标签页一多就找不到，窗口位置每次都要重新调。

DeepSeekChat 把 `chat.deepseek.com` 装进一个原生小壳：**托盘常驻、登录不丢、极致内存**。对话功能 100% 来自官网页面，本项目只做「壳」。**填补了DeepSeek对话没有官方桌面端的空缺**（虽然已经有诸多此类项目了）

## 目录

- [下载](#下载)
- [功能](#功能)
- [内存（实测）](#内存实测)
- [从源码构建](#从源码构建)
- [目录结构](#目录结构)
- [已知限制](#已知限制)
- [License](#license)

## 下载

| 形态 | 文件 | 说明 |
| --- | --- | --- |
| 绿色单 exe | `DeepSeekChat-v0.1.0-portable.exe`（约 6.7MB） | 放任意可写目录直接用，删除即卸载 |
| NSIS 安装包 | `DeepSeekChat-v0.1.0-setup.exe`（约 3.3MB） | 需要开始菜单/卸载入口时使用 |

- 数据存 exe 同目录 `data\`；不可写时自动回退 `%APPDATA%\DeepSeekChat\`。
- 检测不到 WebView2 Runtime 时弹窗征得同意后自动在线安装。

## 功能

**窗口**：官网页面 + 系统标题栏，位置/尺寸/最大化记忆，多显示器拔插自动回正，图片可直接拖入对话。

**刷新（F5 / Ctrl+R）**：刷新重载，登录与上下文不丢；页面右下角也有刷新按钮。

**全屏（F11）**：随时切换。

**对话导出**：页面右下角工具条 → 5 种格式（Markdown / 纯文本 / 网页 / Word / JSON），文件名 `会话名_YYYY-MM-DD_HH-MM-SS`，保存到 `data\Export session\` 并自动打开目录。导出时自动触发官网打印模式全量挂载消息（虚拟滚动下也能导出完整会话），附件格式化为 `📎 文件名 类型 大小`，思考过程与正文严格分离。

**设置**：开机自启、关闭按钮行为（最小化到托盘/直接退出）、极致省内存模式。

**主题跟随**：实测页面背景色，标题栏/边框/设置页/托盘菜单实时同步。

**可靠性**：断网显示本地错误页（原因+重试+20 秒自动重试）；顶层导航白名单（DeepSeek 域内保留，其余交系统浏览器，OAuth 弹窗除外）；渲染进程崩溃 5 秒自动重建；官网「使用环境异常」风控弹窗自动关闭。

**隐私**：零遥测、零上报，数据全部本地。

## 内存（实测）

| 状态 | 预算 | 实测 |
| --- | --- | --- |
| 前台空闲 | ≤100MB | **20MB** |
| 托盘挂起 | ≤30MB | **4MB** |

## 从源码构建

依赖 Windows 10 1809+ x64、Rust stable（MSVC）、Node LTS。

```powershell
.\build-portable.ps1      # 绿色单 exe
.\build-installer.ps1     # NSIS 安装包
```

构建自动重映射本机路径并做隐私扫描（发现本地路径/调试入口/凭据即失败）。开发调试：`cd settings-ui; npm run dev` + `cd src-tauri; cargo run`（校准与 QA 入口见 [PRD.md](PRD.md) 附录 C）。

## 目录结构

```
src-tauri/          Rust 外壳（窗口/托盘/内存/主题/导航/导出/WebView2 引导）
settings-ui/        本地页：设置/托盘菜单/错误页（Vite + TS 无框架）
scripts/            图标生成、NSIS 补丁、发布隐私检查、内存与 UI 的 QA 脚本
build-*.ps1         一键构建
measure-memory.ps1  内存验收
package.json        前端侧工具链（sharp 图标处理，Node ≥18.17）
.github/workflows/  打 v* tag 自动构建并发布 Release
PRD.md              现状规格（每条需求标注实现位置，可对照代码验证）
```

仓库经 Git 分发源码与配置；运行数据（`data\`）、构建产物（`dist\`）都是本机文件，不进仓库。

## 已知限制

- 仅 Windows x64；不能多开（WebView2 用户数据目录硬约束）。
- 主题取色与导出选择器依赖官网当前 DOM，大改版后可能需要同步调整（debug 构建 `DSCHAT_EXPORT_PROBE=1` 可校准）。
- 不做代码签名；界面文案仅中文。
- 开启过开机自启后手动删除程序，注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的 `DeepSeekChat` 键值会残留（指向不存在的路径，无实际影响）。清理：`reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v DeepSeekChat /f`，或「任务管理器 → 启动应用」里关闭。

## License

MIT —— 见 [LICENSE](LICENSE)。图标素材取自 DeepSeek 官网公开 favicon，商标归深度求索官方所有。
