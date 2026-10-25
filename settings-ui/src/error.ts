// 本地错误页（FR-12）：展示失败原因 + 重试按钮 + 自动重试。
// 错误码来自 WebView2 COREWEBVIEW2_WEB_ERROR_STATUS，经 URL fragment 传入。

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

/** WebView2 WEB_ERROR_STATUS → 用户可读的失败原因 */
const REASONS: Record<number, string> = {
  0: "发生未知错误",
  1: "网站证书域名不匹配",
  2: "网站证书已过期",
  3: "客户端证书存在错误",
  4: "网站证书已被吊销",
  5: "网站证书无效",
  6: "无法连接到服务器",
  7: "连接超时",
  8: "连接被中断",
  9: "连接被重置",
  10: "网络已断开",
  11: "无法建立连接",
  12: "域名解析失败（请检查 DNS 或网络）",
  13: "操作已取消",
  14: "重定向失败",
  15: "发生意外错误",
};

const AUTO_RETRY_SECONDS = 20;
/** 倒计时句柄：手动重试时立即停表，避免旧计时器把失败提示清掉 */
let countdownTimer = 0;

function setDark(dark: boolean): void {
  document.documentElement.classList.toggle("dark", dark);
}

function readStatus(): number {
  const hash = window.location.hash.replace(/^#/, "");
  const m = /(?:^|&)status=(-?\d+)/.exec(hash);
  return m ? Number(m[1]) : 0;
}

function render(): void {
  const status = readStatus();
  $("#reason").textContent = REASONS[status] ?? `加载失败（错误码 ${status}）`;
}

function $(sel: string): HTMLElement {
  const el = document.querySelector(sel);
  if (!el) throw new Error(`元素不存在: ${sel}`);
  return el as HTMLElement;
}

function stopCountdown(): void {
  if (countdownTimer) {
    window.clearInterval(countdownTimer);
    countdownTimer = 0;
  }
}

function retry(): void {
  stopCountdown();
  const btn = $("#retry") as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "正在重试…";
  $("#countdown").textContent = "";
  void invoke("retry_navigation").catch(() => {
    btn.disabled = false;
    btn.textContent = "重试";
    $("#countdown").textContent = "重试失败，请检查网络后再试";
  });
}

function startCountdown(): void {
  let remain = AUTO_RETRY_SECONDS;
  const label = $("#countdown");
  label.textContent = `${remain} 秒后自动重试`;
  stopCountdown();
  countdownTimer = window.setInterval(() => {
    remain -= 1;
    if (remain <= 0) {
      retry();
      return;
    }
    label.textContent = `${remain} 秒后自动重试`;
  }, 1000);
}

async function init(): Promise<void> {
  render();
  try {
    const theme = (await invoke("get_theme")) as { dark: boolean };
    setDark(theme.dark);
    await tauri.event.listen("theme-changed", (e: EventPayload) => {
      setDark(!!e.payload?.dark);
    });
  } catch {
    // 主题获取失败时保持浅色
  }

  $("#retry").addEventListener("click", retry);
  // F5 手动重试：与重试按钮同一路径（错误页不是官网，壳的刷新事件不会在这里触发）
  window.addEventListener("keydown", (e) => {
    if (e.key === "F5") {
      e.preventDefault();
      retry();
    }
  });
  startCountdown();
}

void init();
