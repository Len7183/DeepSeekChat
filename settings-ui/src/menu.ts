// 托盘菜单：主题跟随 + 点击转发给 Rust menu_action 命令 + 键盘导航。
// 动画重放：菜单窗常驻复用（hide/show），CSS animation 只在首次加载跑一次，
// 因此每次窗口重新可见时重挂 .pop 类重启入场动画（配合 menu.css 的 #menu.pop）。
export {};

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
  event: { listen: (event: string, handler: (e: EventPayload) => void) => Promise<void> };
}

interface EventPayload {
  payload?: { dark?: boolean };
}

const tauri = (window as unknown as { __TAURI__: TauriGlobal }).__TAURI__;
const invoke = tauri.core.invoke;

function setDark(dark: boolean): void {
  document.documentElement.classList.toggle("dark", dark);
}

/** 重启入场动画：移除类 → 强制回流 → 重新添加 */
function replayEntrance(): void {
  const menu = document.getElementById("menu");
  if (!menu) return;
  menu.classList.remove("pop");
  // 重新添加前必须让浏览器处理一次样式变更，否则类切换会被合并、动画不触发
  void (menu as HTMLElement).offsetWidth;
  menu.classList.add("pop");
}

async function init(): Promise<void> {
  try {
    const theme = (await invoke("get_theme")) as { dark: boolean };
    setDark(theme.dark);
    await tauri.event.listen("theme-changed", (e: EventPayload) => {
      setDark(!!e.payload?.dark);
    });
  } catch {
    // 主题获取失败时保持浅色
  }

  const buttons = Array.from(
    document.querySelectorAll<HTMLButtonElement>("#menu button"),
  );

  // 窗口重显（Rust 侧 menu.show()）→ 重放入场动画。
  // visibilitychange 覆盖 WebView2 窗口 show/hide 的场景（窗口隐藏时页面转入隐藏态）。
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") replayEntrance();
  });
  replayEntrance();

  buttons.forEach((btn) => {
    btn.addEventListener("click", () => {
      void invoke("menu_action", { action: btn.dataset.action }).catch(() => {});
    });
  });

  // 键盘导航（role="menu" 的方向键契约）：↑/↓在菜单项间移动焦点，Home/End 到首尾。
  // 焦点移出后窗口由 Rust 侧守卫收起，这里不做焦点复原。
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      void invoke("menu_action", { action: "hide" }).catch(() => {});
      return;
    }
    const current = document.activeElement as HTMLButtonElement | null;
    const idx = current ? buttons.indexOf(current) : -1;
    let next = -1;
    if (e.key === "ArrowDown") next = (idx + 1 + buttons.length) % buttons.length;
    else if (e.key === "ArrowUp") next = (idx - 1 + buttons.length) % buttons.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = buttons.length - 1;
    if (next >= 0) {
      e.preventDefault();
      buttons[next].focus();
    }
  });
}

void init();
