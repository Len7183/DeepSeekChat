// 设置窗口逻辑：读取/写回 config.json（经 Rust 命令），主题跟随主窗口。
// 交互动效只保留控件本身：分段滑块与开关圆点的拖动跟手 + 点击吸附。
//
// 三层状态，避免快速操作丢意图：
//   confirmed —— 后端已确认的配置（invoke 返回后更新）
//   desired   —— 用户当前期望（乐观更新立刻写入，界面一律按它渲染）
//   pending   —— 保存进行中产生的新意图（下一批随请求发出）
export {};

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
  event: { listen: (event: string, handler: (e: EventPayload) => void) => Promise<void> };
}

interface EventPayload {
  payload?: { dark?: boolean };
}

type CloseAction = "Hide" | "Exit";

interface Config {
  closeAction: CloseAction;
  lowMemoryMode: boolean;
  autostart: boolean;
}

const tauri = (window as unknown as { __TAURI__: TauriGlobal }).__TAURI__;
const invoke = tauri.core.invoke;

let confirmed: Config | null = null;
let desired: Config | null = null;
let pending: Partial<Config> = {};
let inflight = false;
let toastTimer = 0;
/** 拖动中的控件集合：render() 跳过它们，避免保存回包与手势内联样式打架 */
const draggingEls = new Set<HTMLElement>();

/** 拖动行程（px）：按下时圆点拉伸到 19px，行程 = 40-3-3-19=15（CSS .toggle.on.dragging）；
 *  静态圆点 16px 对应行程 18（CSS .toggle.on），由类切换负责，JS 不介入。 */
const KNOB_TRAVEL_PRESSED = 15;

function $(sel: string): HTMLElement {
  const el = document.querySelector(sel);
  if (!el) throw new Error(`元素不存在: ${sel}`);
  return el as HTMLElement;
}

function showToast(text: string): void {
  const toast = $("#toast");
  toast.textContent = text;
  toast.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(hideToast, 5000);
}

function hideToast(): void {
  $("#toast").hidden = true;
}

function renderToggle(el: HTMLElement, on: boolean): void {
  el.classList.toggle("on", on);
  el.setAttribute("aria-checked", String(on));
}

/** 按期望状态渲染全部控件 */
function render(): void {
  if (!desired) return;
  const autoEl = $("#toggle-autostart");
  const lowEl = $("#toggle-lowmem");
  // 拖动中的控件不重渲染：flush 回包会重置类/滑块位置，与手势正在写的
  // 内联 --knob-x/transform 打架（finish/cancel 时自会按期望值校正）
  if (!draggingEls.has(autoEl)) renderToggle(autoEl, desired.autostart);
  if (!draggingEls.has(lowEl)) renderToggle(lowEl, desired.lowMemoryMode);
  $("#seg-hide").setAttribute("aria-checked", String(desired.closeAction === "Hide"));
  $("#seg-exit").setAttribute("aria-checked", String(desired.closeAction === "Exit"));
  // 滑块归位到当前选项（控件尚未初始化时跳过）
  if (segState.thumb && !segState.dragging) {
    segState.place(segState.btnFor(desired.closeAction));
  }
}

/** 记录期望值并尽快落盘：保存中产生的新意图合并进 pending，由 flush 串行发出 */
function submit(patch: Partial<Config>): void {
  if (!desired) return;
  desired = { ...desired, ...patch };
  Object.assign(pending, patch);
  render();
  void flush();
}

async function flush(): Promise<void> {
  if (inflight) return;
  inflight = true;
  try {
    while (Object.keys(pending).length > 0) {
      const patch = pending;
      pending = {};
      try {
        confirmed = (await invoke("set_config", { patch })) as Config;
      } catch (e) {
        showToast(`保存失败：${String(e)}`);
        // 失败字段回滚到已确认值；失败之后产生的新意图照旧保留
        if (confirmed) desired = { ...confirmed, ...pending };
        // 未持久化的新意图不丢：稍后重试一次（窗口关闭前仍有机会落盘），
        // 而不是等一个不相关的下一次用户操作
        if (Object.keys(pending).length > 0) {
          window.setTimeout(() => { void flush(); }, 1200);
        }
        break;
      }
    }
    if (confirmed && Object.keys(pending).length === 0) {
      desired = { ...confirmed };
    }
  } finally {
    inflight = false;
  }
  render();
}

function setDark(dark: boolean): void {
  document.documentElement.classList.toggle("dark", dark);
}

/** 「关闭按钮行为」唯一入口：键盘点按与指针手势共用差异判断，已选值不发请求 */
function setCloseAction(action: CloseAction): void {
  if (!desired) return;
  if (desired.closeAction === action) {
    render();
    return;
  }
  submit({ closeAction: action });
}

// ---------------------------------------------------------------------------
// 分段选择器：点按直接切换；滑块可拖动，松手吸附最近选项
// ---------------------------------------------------------------------------

const segState = {
  thumb: null as HTMLElement | null,
  seg: null as HTMLElement | null,
  buttons: [] as HTMLElement[],
  dragging: false,
  moved: false,
  /** 活动指针：只响应该指针的移动/松手/取消，多指不互相干扰 */
  pointerId: null as number | null,
  startX: 0,
  startTx: 0,
  /** 按下时命中的选项：指针捕获会把 click 重定向到容器，点按以此为准 */
  downBtn: null as HTMLElement | null,
  /** 拖动中胶囊的实际几何（offsetLeft 坐标系），供松手吸附判定用 */
  lastTx: 0,
  lastW: 0,
  /** 手势开始时缓存的两选项几何：拖动热路径不再读 offsetLeft/offsetWidth（避免强制布局） */
  geo: { min: 0, max: 0, wFirst: 0, wLast: 0 },
  /** 当前选中选项的滑块位移（px）：滑块 left:0 锚点与 offsetLeft 同为容器 padding 盒左缘，直接对齐 */
  txFor(btn: HTMLElement): number {
    return btn.offsetLeft;
  },
  widthFor(btn: HTMLElement): number {
    return btn.offsetWidth;
  },
  measure(): void {
    const first = segState.buttons[0];
    const last = segState.buttons[segState.buttons.length - 1];
    if (!first || !last) return;
    segState.geo = {
      min: segState.txFor(first),
      max: segState.txFor(last),
      wFirst: segState.widthFor(first),
      wLast: segState.widthFor(last),
    };
  },
  place(btn: HTMLElement, animate = true): void {
    const { thumb, seg } = segState;
    if (!thumb || !seg) return;
    if (!animate) thumb.style.transition = "none";
    thumb.style.width = `${segState.widthFor(btn)}px`;
    thumb.style.transform = `translateX(${segState.txFor(btn)}px)`;
    if (!animate) requestAnimationFrame(() => (thumb.style.transition = ""));
  },
  btnFor(action: CloseAction): HTMLElement {
    return action === "Exit" ? $("#seg-exit") : $("#seg-hide");
  },
  current(): HTMLElement {
    return desired?.closeAction === "Exit" ? $("#seg-exit") : $("#seg-hide");
  },
};

function setupSegmented(): void {
  const seg = $(".segmented");
  const thumb = document.createElement("div");
  thumb.className = "seg-thumb";
  seg.prepend(thumb);
  segState.seg = seg;
  segState.thumb = thumb;
  segState.buttons = [$("#seg-hide"), $("#seg-exit")];

  // 键盘路径（Enter/空格触发 click）；鼠标点按走下方指针逻辑
  $("#seg-hide").addEventListener("click", () => setCloseAction("Hide"));
  $("#seg-exit").addEventListener("click", () => setCloseAction("Exit"));

  seg.addEventListener("pointerdown", (e) => {
    if (segState.dragging) return; // 已有活动指针，忽略后续指针
    segState.dragging = true;
    segState.moved = false;
    segState.pointerId = e.pointerId;
    segState.startX = e.clientX;
    segState.startTx = segState.txFor(segState.current());
    segState.measure();
    segState.downBtn = (e.target as HTMLElement).closest("button");
    draggingEls.add(seg);
    seg.classList.add("dragging");
    seg.setPointerCapture(e.pointerId);
    e.preventDefault();
  });

  seg.addEventListener("pointermove", (e) => {
    if (!segState.dragging || e.pointerId !== segState.pointerId) return;
    if (!segState.moved && Math.abs(e.clientX - segState.startX) > 4) {
      segState.moved = true;
      // 以激活点为原点：消除触发阈值带来的空程跳变，滑块从当前位置 1:1 跟手
      segState.startX = e.clientX;
    }
    if (!segState.moved) return;
    const { min, max, wFirst, wLast } = segState.geo;
    // 进度 p 同时驱动位移与宽度：胶囊在两个选项之间连续变形，右缘始终落在
    // 两选项右缘的连线上。只位移不变宽（旧写法）会让宽胶囊顶到轨道内缘。
    const raw = segState.startTx + (e.clientX - segState.startX);
    const p = max > min ? Math.min(1, Math.max(0, (raw - min) / (max - min))) : 0;
    const w = wFirst + p * (wLast - wFirst);
    thumb.style.width = `${w}px`;
    thumb.style.transform = `translateX(${min + p * (max - min)}px)`;
    segState.lastTx = min + p * (max - min);
    segState.lastW = w;
  });

  const finish = (): void => {
    if (!segState.dragging) return;
    segState.dragging = false;
    segState.pointerId = null;
    draggingEls.delete(seg);
    seg.classList.remove("dragging");
    let action: CloseAction;
    if (!segState.moved) {
      // 点按：按按下时命中的选项切换
      const target = segState.downBtn ?? segState.current();
      action = target.id === "seg-exit" ? "Exit" : "Hide";
    } else {
      // 吸附：胶囊中心（用拖动中实际几何）距哪个选项中心的行程近就选哪个
      const center = segState.lastTx + segState.lastW / 2;
      const c0 = segState.geo.min + segState.geo.wFirst / 2;
      const c1 = segState.geo.max + segState.geo.wLast / 2;
      action = Math.abs(c0 - center) <= Math.abs(c1 - center) ? "Hide" : "Exit";
    }
    segState.downBtn = null;
    setCloseAction(action);
  };

  // 手势被系统取消（触摸/笔被打断）：只回弹到期望状态，不提交
  const cancel = (): void => {
    if (!segState.dragging) return;
    segState.dragging = false;
    segState.moved = false;
    segState.pointerId = null;
    segState.downBtn = null;
    draggingEls.delete(seg);
    seg.classList.remove("dragging");
    render();
  };

  seg.addEventListener("pointerup", (e) => {
    if (e.pointerId === segState.pointerId) finish();
  });
  seg.addEventListener("pointercancel", (e) => {
    if (e.pointerId === segState.pointerId) cancel();
  });
}

// ---------------------------------------------------------------------------
// 开关：点按切换；圆点可拖动，松手按位置过半吸附、快速甩动按方向提交
// ---------------------------------------------------------------------------

/** 甩动判定速度（px/ms）：松手时的指针速度超过它则按甩动方向提交，不看位置。
 *  参考 iOS UISwitch / Framework7 Toggle 的短促手势处理——轻扫一下就该翻，
 *  不要求拖过中点。 */
const FLICK_V = 0.4;

function setupToggle(el: HTMLElement, key: "autostart" | "lowMemoryMode"): void {
  let dragging = false;
  let moved = false;
  let pointerId: number | null = null;
  let startX = 0;
  let velX = 0; // 指针横向速度（指数滑动平均，px/ms）
  let lastX = 0;
  let lastT = 0;

  const apply = (on: boolean): void => {
    if (!desired) return;
    if (desired[key] === on) {
      render();
      return;
    }
    const patch: Partial<Config> = {};
    patch[key] = on;
    submit(patch);
  };

  el.addEventListener("pointerdown", (e) => {
    if (dragging) return; // 已有活动指针
    dragging = true;
    moved = false;
    pointerId = e.pointerId;
    startX = e.clientX;
    draggingEls.add(el);
    el.setPointerCapture(e.pointerId);
  });

  el.addEventListener("pointermove", (e) => {
    if (!dragging || e.pointerId !== pointerId) return;
    if (!moved && Math.abs(e.clientX - startX) > 4) {
      moved = true;
      // 以激活点为原点：消除触发阈值带来的空程跳变
      startX = e.clientX;
      velX = 0;
      lastX = e.clientX;
      lastT = e.timeStamp;
      el.classList.add("dragging");
    }
    if (!moved) return;
    // 速度估计（EMA）：供松手时的甩动判定
    const dt = e.timeStamp - lastT;
    if (dt > 0) velX = velX * 0.6 + ((e.clientX - lastX) / dt) * 0.4;
    lastX = e.clientX;
    lastT = e.timeStamp;
    const dx = e.clientX - startX;
    const base = desired?.[key] ? KNOB_TRAVEL_PRESSED : 0;
    const x = Math.min(KNOB_TRAVEL_PRESSED, Math.max(0, base + dx));
    el.style.setProperty("--knob-x", `${x}px`);
    // 拖动实时预览：轨道色随进度连续插值（react-switch 式跟手渐变）
    el.style.setProperty("--p", String(x / KNOB_TRAVEL_PRESSED));
    el.classList.toggle("on", x > KNOB_TRAVEL_PRESSED / 2);
  });

  const finish = (): void => {
    if (!dragging) return;
    dragging = false;
    pointerId = null;
    draggingEls.delete(el);
    el.classList.remove("dragging"); // 恢复弹簧过渡
    // 目标类先同步就位（内联 --knob-x 仍在生效，视觉不变），再释放内联值，
    // 圆点从当前位置一步弹簧到位；若先释放再等保存往返切类，会先朝旧位回弹再折返。
    const settle = (on: boolean): void => {
      el.classList.toggle("on", on);
      el.setAttribute("aria-checked", String(on));
    };
    if (!moved) {
      // 点按：乐观更新即时吸附，不等保存往返
      const next = !(desired?.[key] ?? false);
      settle(next);
      apply(next);
      return;
    }
    const x = parseFloat(el.style.getPropertyValue("--knob-x")) || 0;
    // 松手判定：快速甩动按方向提交（短促手势方向优先于位置），否则按位置过半
    let on = x > KNOB_TRAVEL_PRESSED / 2;
    if (velX > FLICK_V) on = true;
    else if (velX < -FLICK_V) on = false;
    settle(on);
    // 目标类已就位，释放内联值：圆点与轨道色从当前位置一步过渡到目标
    el.style.removeProperty("--knob-x");
    el.style.removeProperty("--p");
    apply(on); // 持久化；保存失败时 flush() 的回滚会把圆点弹回已确认状态
  };

  // 手势被系统取消：回弹到期望状态，不提交
  const cancel = (): void => {
    if (!dragging) return;
    dragging = false;
    moved = false;
    pointerId = null;
    draggingEls.delete(el);
    el.classList.remove("dragging");
    render();
    el.style.removeProperty("--knob-x");
    el.style.removeProperty("--p");
  };

  el.addEventListener("pointerup", (e) => {
    if (e.pointerId === pointerId) finish();
  });
  el.addEventListener("pointercancel", (e) => {
    if (e.pointerId === pointerId) cancel();
  });

  // 键盘（role=switch）
  el.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      apply(!(desired?.[key] ?? false));
    }
  });
}

/** 后端载荷的入口校验：契约漂移/畸形响应在此拦截，不让它污染 confirmed
 *  （后续所有回滚与保存都会基于 confirmed 传播，坏数据会扩散到全状态模型） */
function asConfig(v: unknown): Config | null {
  if (typeof v !== "object" || v === null) return null;
  const c = v as Record<string, unknown>;
  if (
    (c.closeAction !== "Hide" && c.closeAction !== "Exit") ||
    typeof c.lowMemoryMode !== "boolean" ||
    typeof c.autostart !== "boolean"
  ) {
    return null;
  }
  return { closeAction: c.closeAction, lowMemoryMode: c.lowMemoryMode, autostart: c.autostart };
}

async function init(): Promise<void> {
  try {
    const raw = await invoke("get_config");
    const parsed = asConfig(raw);
    if (!parsed) throw new Error("配置载荷校验失败");
    confirmed = parsed;
    desired = { ...confirmed };
    // 主题先订阅后读：先注册 listener 再取当前值，两步之间发生的主题切换
    // 也会被 listener 捕获（反序会漏掉一次切换，页面卡在旧主题直到下次变化）
    let themeDark = false;
    await tauri.event.listen("theme-changed", (e: EventPayload) => {
      setDark(!!e.payload?.dark);
    });
    const theme = (await invoke("get_theme")) as { dark: boolean };
    themeDark = !!theme?.dark;
    setDark(themeDark);

    const dir = (await invoke("get_data_dir")) as string;
    $("#data-dir").textContent = `数据目录：${dir}`;
  } catch (e) {
    showToast(`初始化失败：${String(e)}`);
    return;
  }

  // 控件装配同样受保护：单个元素缺失（$() 抛错）不应变成未处理的
  // Promise 拒绝 + 半初始化静默 UI
  try {
    render();
    setupSegmented();
    setupToggle($("#toggle-autostart"), "autostart");
    setupToggle($("#toggle-lowmem"), "lowMemoryMode");
    // 布局就绪后放置滑块（无动画初始定位）
    requestAnimationFrame(() => {
      segState.place(segState.current(), false);
      // 字体加载可能改变宽度，再校正一次
      document.fonts?.ready.then(() => segState.place(segState.current(), false));
    });

    // 数据目录：单击打开
    $("#data-dir").addEventListener("click", () => {
      void invoke("open_data_dir").catch((e) => showToast(`打开失败：${String(e)}`));
    });

    // 项目主页（关于DeepSeekChat）
    $("#about").addEventListener("click", () => {
      void invoke("open_project_home").catch((e) => showToast(`打开失败：${String(e)}`));
    });
  } catch (e) {
    showToast(`界面装配失败：${String(e)}`);
  }
}

void init().catch((e) => showToast(`初始化异常：${String(e)}`));
