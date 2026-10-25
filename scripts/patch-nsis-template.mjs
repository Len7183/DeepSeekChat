// 对 Tauri 官方 NSIS 模板（tauri-cli-v2.12.0 tag）做定制补丁（FR-15）：
// 1. Finish 页新增「创建开始菜单快捷方式」勾选框（默认不创建）
// 2. 卸载勾选框语义反转为「保留用户数据」，默认（不勾）删除：
//    exe 同目录 data\ 与 %APPDATA%\DeepSeekChat 中实际存在者
// 用法：node scripts/patch-nsis-template.mjs <模板路径>
import { readFileSync, writeFileSync } from "node:fs";

const target = process.argv[2] ?? "src-tauri/nsis/installer.nsi";
let s = readFileSync(target, "utf8");
let applied = 0;

function patch(name, from, to) {
  if (s.includes(to)) {
    console.log(`跳过（已应用）: ${name}`);
    return;
  }
  if (!s.includes(from)) {
    console.error(`未找到锚点: ${name}`);
    process.exitCode = 1;
    return;
  }
  // 函数形式替换：NSIS 文本满是 $ 变量，字符串形式的 to 会把 $&/$`/$'/$$
  // 当作特殊替换模式静默改写
  s = s.replace(from, () => to);
  applied++;
  console.log(`已应用: ${name}`);
}

// ---- 1. 新增变量（须在使用点之前：NSIS 单遍编译）----
patch(
  "vars",
  "Var OldMainBinaryName",
  "Var OldMainBinaryName\n" +
    "; DeepSeekChat: 安装向导附加勾选（开始菜单快捷方式默认不创建）\n" +
    "Var StartMenuShortcutCheckbox\nVar StartMenuShortcutState"
);

// ---- 2. Finish 页挂钩 + 开始菜单勾选框 ----
patch(
  "finish-page",
  "; Show run app after installation.\n" +
    "!define MUI_FINISHPAGE_RUN\n" +
    "!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary\n" +
    "!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive\n" +
    "!insertmacro MUI_PAGE_FINISH",
  "; Show run app after installation.\n" +
    "!define MUI_FINISHPAGE_RUN\n" +
    "!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary\n" +
    "!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive\n" +
    "!define MUI_PAGE_CUSTOMFUNCTION_SHOW FinishShow\n" +
    "!define MUI_PAGE_CUSTOMFUNCTION_LEAVE FinishLeave\n" +
    "!insertmacro MUI_PAGE_FINISH\n" +
    "\n" +
    "Function FinishShow ; DeepSeekChat: finish 页附加「开始菜单快捷方式」勾选\n" +
    "  ; 必须用 nsDialogs：MUI2 在 nsDialogs::Create 之后、nsDialogs::Show 之前调用本页\n" +
    "  ; SHOW 函数，此时创建才会挂进同一个页面对话框，并共享 'u' 单位的 DPI 缩放与默认字体。\n" +
    "  ; 坐标沿用 MUI2 完成页那一列（x=120u、宽 195u、高 10u）：「运行」90u、「桌面快捷方式」110u。\n" +
    "  ${NSD_CreateCheckbox} 120u 130u 195u 10u \"$(createStartMenu)\"\n" +
    "  Pop $StartMenuShortcutCheckbox\n" +
    "  SetCtlColors $StartMenuShortcutCheckbox \"${MUI_TEXTCOLOR}\" \"${MUI_BGCOLOR}\"\n" +
    "FunctionEnd\n" +
    "\n" +
    "Function FinishLeave\n" +
    "  SendMessage $StartMenuShortcutCheckbox ${BM_GETCHECK} 0 0 $StartMenuShortcutState\n" +
    "  ; 开始菜单快捷方式（默认不创建，勾选才建）\n" +
    "  ${If} $StartMenuShortcutState = 1\n" +
    "    !insertmacro MUI_STARTMENU_WRITE_BEGIN Application\n" +
    "      Call CreateOrUpdateStartMenuShortcut\n" +
    "    !insertmacro MUI_STARTMENU_WRITE_END\n" +
    "  ${EndIf}\n" +
    "FunctionEnd"
);

// ---- 3. 安装节内不再无条件创建开始菜单快捷方式（移至 FinishLeave）----
// 静默安装（/S）与 passive 模式不经过 finish 页（SkipIfPassive 跳过 PRE，
// /S 连页面都不建），FinishLeave 不会执行——开始菜单快捷方式不创建。
// 这与「默认不创建」的设计语义一致：无人勾选即不建。
patch(
  "start-menu-off",
  "  ; Create start menu shortcut\n" +
    "  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application\n" +
    "    Call CreateOrUpdateStartMenuShortcut\n" +
    "  !insertmacro MUI_STARTMENU_WRITE_END",
  "  ; 开始菜单快捷方式改为 finish 页勾选后创建（默认不创建；\n" +
    "  ; 静默/passive 安装无 finish 页，等价于未勾选，同样不创建）"
);

// ---- 4. 卸载：默认删除用户数据，勾选「同时保留用户数据」时跳过 ----
patch(
  "uninstall-data",
  "  ${If} $DeleteAppDataCheckboxState = 1\n" +
    "  ${AndIf} $UpdateMode <> 1\n" +
    "    ; Clear the install location $INSTDIR from registry\n" +
    "    DeleteRegKey SHCTX \"${MANUPRODUCTKEY}\"\n" +
    "    DeleteRegKey /ifempty SHCTX \"${MANUKEY}\"\n" +
    "\n" +
    "    ; Clear the install language from registry\n" +
    "    DeleteRegValue HKCU \"${MANUPRODUCTKEY}\" \"Installer Language\"\n" +
    "    DeleteRegKey /ifempty HKCU \"${MANUPRODUCTKEY}\"\n" +
    "    DeleteRegKey /ifempty HKCU \"${MANUKEY}\"\n" +
    "\n" +
    "    SetShellVarContext current\n" +
    "    RmDir /r \"$APPDATA\\${BUNDLEID}\"\n" +
    "    RmDir /r \"$LOCALAPPDATA\\${BUNDLEID}\"\n" +
    "  ${EndIf}",
  "  ${If} $DeleteAppDataCheckboxState = 1\n" +
    "  ${AndIf} $UpdateMode <> 1\n" +
    "    ; Clear the install location $INSTDIR from registry\n" +
    "    DeleteRegKey SHCTX \"${MANUPRODUCTKEY}\"\n" +
    "    DeleteRegKey /ifempty SHCTX \"${MANUKEY}\"\n" +
    "\n" +
    "    ; Clear the install language from registry\n" +
    "    DeleteRegValue HKCU \"${MANUPRODUCTKEY}\" \"Installer Language\"\n" +
    "    DeleteRegKey /ifempty HKCU \"${MANUPRODUCTKEY}\"\n" +
    "    DeleteRegKey /ifempty HKCU \"${MANUKEY}\"\n" +
    "\n" +
    "    ; DeepSeekChat：勾选「保留用户数据」——注册表清理照常（卸载必要），\n" +
    "    ; 但不再删除任何数据目录（原模板此处 RmDir 的 ${BUNDLEID} 目录若被\n" +
    "    ; WebView2 等组件写入过，恰是登录态所在，勾选保留时删它自相矛盾）\n" +
    "  ${EndIf}\n" +
    "\n" +
    "  ; DeepSeekChat：默认（未勾选「保留」）删除用户数据\n" +
    "  ; 实际数据目录 = exe 同目录 data\\（不可写时回退 %APPDATA%\\DeepSeekChat）；\n" +
    "  ; BUNDLEID 目录一并清理（个别组件可能按标识符写入）\n" +
    "  ${If} $DeleteAppDataCheckboxState <> 1\n" +
    "  ${AndIf} $UpdateMode <> 1\n" +
    "    RmDir /r \"$INSTDIR\\data\"\n" +
    "    RmDir /r \"$APPDATA\\DeepSeekChat\"\n" +
    "    SetShellVarContext current\n" +
    "    RmDir /r \"$APPDATA\\${BUNDLEID}\"\n" +
    "    RmDir /r \"$LOCALAPPDATA\\${BUNDLEID}\"\n" +
    "    SetShellVarContext current\n" +
    "  ${EndIf}"
);

// ---- 5. 卸载勾选框文案改为「保留用户数据」 + 新勾选框的多语言文案 ----
patch(
  "lang-strings",
  "Function .onInit",
  "; DeepSeekChat 自定义文案（置于语言声明之后以覆盖/补充默认值）\n" +
    "LangString deleteAppData ${LANG_SIMPCHINESE} \"保留用户数据（聊天登录态、设置与日志）\"\n" +
    "LangString deleteAppData ${LANG_ENGLISH} \"Keep user data (login session, settings & logs)\"\n" +
    "LangString createStartMenu ${LANG_SIMPCHINESE} \"创建开始菜单快捷方式\"\n" +
    "LangString createStartMenu ${LANG_ENGLISH} \"Create Start Menu shortcut\"\n" +
    "\n" +
    "Function .onInit"
);

if (process.exitCode) {
  // 锚点缺失 = 模板版本漂移：不写回半成品（部分补丁可能引用未定义变量，
  // 让 NSIS 编译期才炸），直接失败让构建停止
  console.error("存在未命中的锚点，已放弃写回（保留原模板），请检查模板版本是否为 tauri-cli-v2.12.0");
  process.exit(1);
}
writeFileSync(target, s);
console.log(`完成：应用 ${applied} 处补丁`);
