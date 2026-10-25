// 注入主窗口的侦测脚本（FR-10 主题跟随 / FR-09 空闲判定 / FR-18 风控弹窗 / FR-21 刷新）：
// 1. 侦测官网深浅主题 + 实测页面背景色 → 经事件通道上报 `dschat://theme`
//    （对标 deepseek-ai/deepseek-harness 的两段式：启动回退色 + 页面实测色随时替换）
// 2. 用户活跃心跳 → 事件通道上报 `dschat://heartbeat`（空闲修剪的判定依据）
// 3. 自动关闭官网「使用环境异常」风控弹窗，结果经 `dschat://log` 记录
// 4. 上报一律走 `plugin:event|emit`，不直接 invoke 应用命令——远程源过不了 IPC 的 ACL
//    （tauri 在 webview 分发处对非本地来源直接拒绝未授权命令），事件权限才是唯一通路。
// 5. 日志脱敏原则：脚本不上报任何 URL / Cookie / 输入内容

pub const INIT_JS: &str = r#"
(function () {
  if (window.__DSCHAT_INJECTED__) return;
  window.__DSCHAT_INJECTED__ = true;

  var lastSent = '';
  var lastActive = 0;

  function parseRgb(str) {
    var m = /rgba?\(([^)]+)\)/.exec(str || '');
    if (!m) return null;
    // 兼容两种序列化：旧式逗号分隔 'rgb(255, 255, 255)' 与 CSS Color 4
    // 空格式 'rgb(255 0 0)' / 'rgb(0 0 0 / 0.5)'（Chromium ~M121 起
    // getComputedStyle 返回空格式，不兼容则主题检测整体失效）
    var parts = m[1].trim().split(/[\s,\/]+/).map(function (s) { return parseFloat(s); });
    if (parts.length < 3) return null;
    return { r: parts[0], g: parts[1], b: parts[2], a: parts.length > 3 ? parts[3] : 1 };
  }

  function luminance(c) {
    return (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b) / 255;
  }

  function toHex(c) {
    function h(v) { var s = Math.round(v).toString(16); return s.length < 2 ? '0' + s : s; }
    return '#' + h(c.r) + h(c.g) + h(c.b);
  }

  // 候选链：html → body → 前 5 层应用容器，取第一个非透明背景色。
  // strict=true 时不得回退全扫（调用方用于 1s 轮询的廉价探测）。
  function pageBackground(strict) {
    var chain = [];
    if (document.documentElement) chain.push(document.documentElement);
    var body = document.body;
    if (body) {
      chain.push(body);
      var el = body.firstElementChild;
      for (var i = 0; i < 5 && el; i++) {
        chain.push(el);
        el = el.firstElementChild;
      }
    }
    for (var i = 0; i < chain.length; i++) {
      var c = null;
      try { c = parseRgb(getComputedStyle(chain[i]).backgroundColor); } catch (e) {}
      if (c && c.a > 0.9) return c;
    }
    if (strict) return null;
    return frequencyBackground();
  }

  // 兜底：扫描页面元素取最高频的非透明背景色（SPA 容器较深时）
  function frequencyBackground() {
    var nodes, counts = {}, seen = {};
    try {
      nodes = document.querySelectorAll('body, body div, body main, body section, body aside, body header');
    } catch (e) { return null; }
    var limit = Math.min(nodes.length, 800);
    for (var i = 0; i < limit; i++) {
      var c = null;
      try { c = parseRgb(getComputedStyle(nodes[i]).backgroundColor); } catch (e) {}
      if (!c || c.a <= 0.9) continue;
      var key = toHex(c);
      counts[key] = (counts[key] || 0) + 1;
      seen[key] = c; // 直接留颜色对象，省掉 hex → rgb 字符串 → 再解析的回程
    }
    var best = null, bestN = 0;
    for (var k in counts) {
      if (counts[k] > bestN) { best = k; bestN = counts[k]; }
    }
    return best ? seen[best] : null;
  }

  function detect(strict) {
    var c = pageBackground(strict);
    if (!c) return null;
    return { dark: luminance(c) < 0.55, hex: toHex(c) };
  }

  // force 省略/false = 全量测量（候选链全透明时回退全扫，最多 800 节点）；
  // force=true = 廉价探测（strict：只测候选链不回退），1s 轮询专用，
  // 避免大 DOM 上每秒一次全扫。全扫机会留给 DOM 事件路径。
  function send(force) {
    var d;
    try { d = detect(force); } catch (e) { return; }
    if (!d) return;
    var key = d.dark + '|' + d.hex;
    // 同帧广播给页面内其他注入组件（导出工具条订阅此信号，与标题栏同一时刻变色）。
    // 与 IPC 同 key 去重：属性高频变化页（打字指示器/流式渲染）不再每批派发事件。
    if (key === lastSent) return;
    lastSent = key;
    try { document.dispatchEvent(new CustomEvent('dschat-theme', { detail: { dark: d.dark, hex: d.hex } })); } catch (e) {}
    try {
      // 远程源不可直接调用应用命令（ACL），用事件插件上报（core:event:default 已授权）
      window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {
        event: 'dschat://theme',
        payload: { dark: d.dark, bg: d.hex }
      });
    } catch (e) {}
  }

  // 官网风控弹窗（「使用环境异常」）自动关闭：定位标题 → 点同弹窗内的关闭图标钮；
  // 仅当确实找不到任何关闭钮时才移除挂载点（保守兜底）。加载后 90 秒内递增间隔
  // 轮询 + DOM 变化即扫；90 秒后仍有低频兜底（1 次/10s，迟挂载的弹窗也能关掉）。
  // 标题匹配用完整已知文案，不做裸「环境/风险」子串宽匹配——那会把隐私提示、
  // 网络设置等正当弹窗一并关掉。
  var sweepDeadline = Date.now() + 90000;

  function emitLog(msg) {
    try {
      window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {
        event: 'dschat://log',
        payload: { msg: msg }
      });
    } catch (e) {}
  }

  function dismissRiskDialog() {
    try {
      var titles = document.querySelectorAll('.ds-modal-content__title');
      for (var i = 0; i < titles.length; i++) {
        var t = (titles[i].textContent || '').trim();
        // 完整标题匹配（官网实测文案「使用环境异常」开头）
        if (t.indexOf('使用环境异常') !== 0) continue;
        var modal = titles[i].closest('.ds-modal-content');
        if (!modal) continue;
        var btns = modal.querySelectorAll('[role="button"]');
        var closed = false;
        for (var j = btns.length - 1; j >= 0; j--) {
          var cls = String(btns[j].className || '');
          if (cls.indexOf('ds-button--icon') >= 0) { btns[j].click(); closed = true; break; }
        }
        if (!closed) {
          // 移除是最后手段：绕过站点框架的挂载管理（React reconciliation 可能
          // 抛错），只在连一个可点关闭钮都没有时使用，并单独留痕区分
          var wrapper = modal.closest('.ds-theme.ds-modal-wrapper') || modal.closest('.ds-modal');
          if (wrapper && wrapper.parentElement) {
            wrapper.parentElement.removeChild(wrapper);
            emitLog('risk-dialog-removed');
            return true;
          }
        }
        if (closed) { emitLog('risk-dialog-dismissed'); return true; }
      }
    } catch (e) {}
    return false;
  }

  function sweepRiskDialog() {
    if (Date.now() > sweepDeadline) {
      // 90 秒后低频兜底：迟挂载/重现的风控弹窗仍会被关掉（10s 一次，开销可忽略）
      setTimeout(function lateSweep() {
        dismissRiskDialog();
        setTimeout(lateSweep, 10000);
      }, 10000);
      return;
    }
    if (dismissRiskDialog()) return;
    // 递增间隔：加载高峰期 500ms 快扫，随后 1s/1.5s 递减频率（弹窗只在全新环境出现，
    // 稍晚关闭无感知；观察器路径仍会即时补扫）
    var el = Date.now() - sweepStart;
    setTimeout(sweepRiskDialog, el < 8000 ? 500 : el < 20000 ? 1000 : 1500);
  }
  var sweepStart = Date.now();

  // 深浅色跟随（按钮同款方法，实测最可靠）：
  // 1) 观察器不筛属性类型（class / data-* / 任意属性全覆盖）+ subtree 全域，
  //    回调内直接同帧测量上报（不排队 rAF——排队会把首测推迟到下一帧）；
  // 2) 变化确认后 rAF 逐帧跟踪（官网切换自带 ~1s CSS 过渡，逐帧跟住过渡色），
  //    send() 按 key 去重，颜色未变不产生 IPC；
  // 3) 1s 轮询兜底：无论官网以何种方式改主题（含换样式表等观察不到的路径），
  //    最迟 1s 跟上；单次测量仅候选链几次 getComputedStyle，开销可忽略。
  var trackUntil = 0;
  var burstUntil = 0;
  var tracking = false;

  function trackTheme() {
    var now = Date.now();
    trackUntil = now + 1000;
    if (tracking) return;
    burstUntil = now + 3000;
    tracking = true;
    requestAnimationFrame(function step() {
      send();
      var t = Date.now();
      if (t < trackUntil && t < burstUntil) {
        requestAnimationFrame(step);
      } else {
        tracking = false;
      }
    });
  }

  var riskLast = 0;
  // DOM 事件路径的测量合帧：MutationObserver 微任务批可能一帧多次回调
  // （打字指示器/流式渲染的 class churn），同步 send() 每次都是候选链
  // getComputedStyle（候选链全透明时还会全扫 800 节点）。同一帧只测一次，
  // 主题切换的响应不受影响（变化仍在本帧被发现）。
  var frameQueued = false;
  function onPageChange() {
    // 风控弹窗补扫限频 1 次/秒（主轮询兜底，这里只做即时性）
    var now = Date.now();
    if (now <= sweepDeadline && now - riskLast > 1000) {
      riskLast = now;
      dismissRiskDialog();
    }
    // 同帧直测（按钮方法）+ 逐帧跟踪：合并到下一帧前只测一次
    if (frameQueued) return;
    frameQueued = true;
    requestAnimationFrame(function () {
      frameQueued = false;
      send();
      trackTheme();
    });
  }

  function start() {
    send();
    sweepRiskDialog();
    try {
      var obs = new MutationObserver(onPageChange);
      // attributes + childList：弹窗可能经纯 DOM 插入出现（无属性变化），
      // 只盯属性会漏掉迟到挂载的风控弹窗
      obs.observe(document.documentElement, { subtree: true, attributes: true, childList: true });
    } catch (e) {}
    // 轮询兜底：任何观察不到的主题变化路径最迟 1s 跟上（strict 廉价探测，
    // 不触发全扫；DOM 事件路径仍全量测量，候选链全透明时仍有全扫兜底机会）
    setInterval(function () { send(true); }, 1000);
    // 应用挂载较晚的兜底
    setTimeout(send, 1500);
    setTimeout(send, 4000);
  }

  // 刷新（FR-21）：WebView2 的浏览器加速键被 wry 默认关闭，F5/Ctrl+R 会落到页面，
  // 这里拦截后经事件通道让壳执行真刷新（保留 SPA 状态的 ICoreWebView2::Reload）。
  // Ctrl+F5 强刷走同一通道——WebView2 Reload 不带绕缓存语义，官网静态资源自带
  // 版本化 URL，无需真正的绕缓存刷新。
  // 快捷键事件上报（emit 失败时不吞）：返回是否成功，供调用方决定是否留痕。
  // preventDefault 只在上报尝试前调用一次；若 ACL/通道异常，至少日志里能看到。
  function emitKey(event, payload) {
    try {
      window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {
        event: event, payload: payload || {}
      });
      return true;
    } catch (err) {
      // 通道故障留痕（debug 构建可见；release 侧由 dschat://log 脱敏策略控制）
      try { console.warn('[dschat] emit ' + event + ' failed', err); } catch (e2) {}
      return false;
    }
  }

  window.addEventListener('keydown', function (e) {
    var k = (e.key || '').toLowerCase();
    if (k === 'f5' || (e.ctrlKey && k === 'r')) {
      e.preventDefault();
      emitKey('dschat://reload');
    } else if (k === 'f11') {
      // F11 全屏（FR-23）：与刷新同通道模式，壳侧 set_fullscreen 翻转
      e.preventDefault();
      emitKey('dschat://fullscreen');
    }
  }, true);

  // 心跳：节流 30s，仅表明"用户活跃"，不含任何内容
  ['pointermove', 'pointerdown', 'keydown', 'wheel'].forEach(function (ev) {
    window.addEventListener(ev, function () {
      var now = Date.now();
      if (now - lastActive < 30000) return;
      lastActive = now;
      try {
        window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {
          event: 'dschat://heartbeat',
          payload: {}
        });
      } catch (e) {}
    }, { passive: true });
  });

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
"#;
