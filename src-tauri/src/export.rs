// 对话导出（FR-22）：
// 右下角悬浮工具条（刷新 + 导出）→ 导出展开格式菜单（Markdown / TXT / HTML / JSON /
// Word / PDF）→ 页面抓取当前对话 → `dschat://export` 事件（FR-20 唯一通路）→
// 壳侧落盘或打印。设计要点：
// - DOM 抓取**自适应**：不押注具体类名，先试探「用户消息锚点类」（官网用户气泡的
//   ds-user-* 体系），命中则以最近公共容器为一轮；全部失手再退回 Markdown 容器计数。
//   探针（DSCHAT_EXPORT_PROBE=1）回传命中详情供改版校准。
// - 全链路零依赖：格式转换手写；PDF 走壳侧 WebView2 原生 PrintToPdf（所见即所得）；
//   Word 导出 Word 可直接打开的 HTML（.doc 单文件，MHT 语义）——不引入 docx 生成器。
// - 上报载荷仅纯文本，日志只记文件名（FR-14）。

use serde_json::Value;

use crate::config::atomic_write;

/// 导出脚本：悬浮工具条 + 抓取 + 上报。主题跟随与壳同源（实测背景亮度）。
pub const EXPORT_JS: &str = r#"
(function () {
  if (window.__DSCHAT_EXPORT__) return;
  window.__DSCHAT_EXPORT__ = true;

  // ---------- 主题跟随：订阅标题栏同源信号（同帧变色）+ 自测兜底 ----------
  // 速度对齐标题栏的正确姿势：标题栏快是因为 injected.js 的 rAF 同帧测量链——
  // 工具条自己另测必然慢半拍（CSS 过渡色导致阈值跨越滞后）。injected.js 每次
  // 测量后派发 document 级 dschat-theme 事件，这里监听即变（含过渡期逐帧事件，
  // 天然跟住过渡色）；仅当信号不可用时才退回自测 + 1s 轮询。
  var lastSignalAt = 0;
  function parseRgb(str) {
    var m = /rgba?\(([^)]+)\)/.exec(str || '');
    if (!m) return null;
    var p = m[1].split(',').map(function (s) { return parseFloat(s); });
    return p.length >= 3 ? { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 } : null;
  }
  function measureDark() {
    var chain = [document.documentElement, document.body];
    var el = document.body && document.body.firstElementChild;
    for (var i = 0; i < 3 && el; i++) { chain.push(el); el = el.firstElementChild; }
    for (var i = 0; i < chain.length; i++) {
      var c = null;
      try { c = parseRgb(getComputedStyle(chain[i]).backgroundColor); } catch (e) {}
      if (c && c.a > 0.9) return (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b) / 255 < 0.55;
    }
    return false;
  }
  function setDark(d) {
    // 无条件 toggle：值未变时 toggle 本身是廉价 no-op，但保证任何路径
    // （事件/轮询/mount 重挂）调用后 class 必然与状态一致——若按
    // 「值未变就返回」提前退出，刷新场景（事件早于 body 就绪）mount 后
    // 永远补不上 class，深色页按钮卡浅色。
    var bar = document.getElementById('dschat-bar');
    if (bar) bar.classList.toggle('dschat-dark', d);
  }
  function applyTheme() { setDark(measureDark()); }  // mount 时初值；运行期由事件/轮询驱动
  // 主路径：标题栏同源信号。监听先于一切初始化挂上；bar 尚未 mount 时事件到达，
  // 先记入 pendingDark（mount 完成即应用）——刷新场景 injected.js 首批事件早于
  // body 就绪，错过即卡浅色（class 初值缺失）。
  var pendingDark = null;
  try {
    document.addEventListener('dschat-theme', function (e) {
      // 仅有效事件刷新活跃时间戳：无效载荷不应阻塞自测兜底
      if (!(e && e.detail && typeof e.detail.dark === 'boolean')) return;
      lastSignalAt = Date.now();
      pendingDark = e.detail.dark;
      setDark(e.detail.dark);
    }, false);
  } catch (e) {}
  // 兜底：初值自测 + 1s 轮询。事件源活跃（60s 内有信号）时轮询让位
  // （标题栏结论更可靠）；pendingDark 同样只在信号新鲜期内生效——
  // 事件源死亡后自测恢复，避免陈旧缓冲值把主题永久钉死。
  setDark(measureDark());
  setInterval(function () {
    var live = Date.now() - lastSignalAt < 60000;
    if (pendingDark !== null && live) { setDark(pendingDark); return; }
    if (live) return;
    setDark(measureDark());
  }, 1000);

  // ---------- 样式：主题全部走 CSS 变量，.dschat-dark 在根容器上一处切换 ----------
  // 菜单是 #dschat-bar 的子元素（变量经继承生效；旧版菜单挂在 body 下继承不到，永远浅色）
  function css() {
    return '#dschat-bar{--xb-bg:rgba(255,255,255,.92);--xb-border:rgba(0,0,0,.12);--xb-fg:#4d6bfe;'
 +'--mi-bg:#ffffff;--mi-border:rgba(0,0,0,.1);--mi-fg:#333a45;--mi-hover:rgba(77,107,254,.08);'
 +'--mi-key:#98a0ad;--menu-shadow:0 6px 24px rgba(0,0,0,.16);'
 +'position:fixed;right:16px;bottom:16px;'
 +'z-index:2147483000;display:flex;flex-direction:column;gap:8px;align-items:flex-end;'
 +'font-family:system-ui,sans-serif}'
 +'#dschat-bar.dschat-dark{--xb-bg:rgba(42,42,46,.92);--xb-border:rgba(255,255,255,.16);'
 +'--mi-bg:#2b2b2f;--mi-border:rgba(255,255,255,.14);--mi-fg:#e8eaf0;--mi-hover:rgba(255,255,255,.09);'
 +'--mi-key:#8f96a3;--menu-shadow:0 6px 24px rgba(0,0,0,.55)}'
+'#dschat-export-menu{position:absolute;right:50px;bottom:-4px;z-index:2147483001;visibility:hidden;'
+'opacity:0;transform:translateY(6px) scale(.97);transform-origin:right bottom;display:flex;flex-direction:column;'
+'background:var(--mi-bg);border:1px solid var(--mi-border);border-radius:10px;'
+'box-shadow:var(--menu-shadow);overflow:hidden;width:max-content;min-width:150px;padding:6px;'
+'transition:opacity .18s ease,transform .18s ease,visibility 0s linear .18s}'
+'#dschat-export-menu.open{visibility:visible;opacity:1;transform:none;transition:opacity .18s ease,transform .18s ease}'
+'.dschat-mi{display:flex;align-items:center;flex:1 1 0;min-height:38px;padding:10px 14px;border:0;background:none;'
+'cursor:pointer;font:13px/1 system-ui,sans-serif;color:var(--mi-fg);text-align:left;white-space:nowrap;box-sizing:border-box;border-radius:6px}'
+'.dschat-mi:hover{background:var(--mi-hover)}'
+'.dschat-xbtn{width:38px;height:38px;border-radius:50%;border:1px solid var(--xb-border);cursor:pointer;'
+'display:flex;align-items:center;justify-content:center;background:var(--xb-bg);color:var(--xb-fg);'
+'opacity:.5;padding:0;transition:opacity .15s,transform .26s cubic-bezier(.34,1.56,.64,1),opacity .26s}'
+'.dschat-xbtn:hover{opacity:1}'
+'.dschat-xbtn:active{transform:scale(.92)}'
+'.dschat-xbtn svg{width:19px;height:19px;pointer-events:none}'
+'#dschat-bar.dschat-hidden{opacity:0;pointer-events:none;transform:translateY(10px)}'
+'#dschat-bar{transition:opacity .26s ease,transform .26s cubic-bezier(.34,1.56,.64,1)}';
  }
  function ensureStyle() {
    if (document.getElementById('dschat-style')) return;
    var s = document.createElement('style');
    s.id = 'dschat-style';
    s.textContent = css();
    document.head.appendChild(s);
  }

  function emit(event, payload) {
    try {
      // invoke 返回 Promise：同步 try/catch 只兜 internals 缺失，异步拒绝
      // （插件拒绝/通道失败）用 .catch 静默吞掉——导出无 UI 反馈（用户要求），
      // 落盘失败时 Rust 侧日志仍会记录
      window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event: event, payload: payload })
        .catch(function () {});
    } catch (e) {}
  }

  function qsa(root, sel) {
    try { return root ? Array.prototype.slice.call(root.querySelectorAll(sel)) : []; }
    catch (e) { return []; }
  }

  // ---------- DOM 抓取（2026-10 实测官网结构，见 EXPORT_PROBE_JS）----------
  // 消息轮单元：.ds-message。用户轮 = 不含 .ds-markdown 的 .ds-message（纯文本气泡，
  // 实测类 "d29f3d7d ds-message _63c77b1"）；助手轮 = 含 .ds-markdown（回答正文
  // .ds-markdown.ds-assistant-message-main-content）与可选思考块 .ds-think-content。
  // 智能取文本：附件气泡里「文件名/类型标签/大小」是相邻的独立元素（如
  // DIV.eafda4ae），裸 textContent 会挤成 "xxx.txtTXT 56.01KB"——遍历子树，
  // 元素边界处补空格（块级元素补换行），再压缩多余空白。
  function nodeText(el) {
    if (!el) return '';
    function walk(node, out) {
      for (var n = node.firstChild; n; n = n.nextSibling) {
        if (n.nodeType === 3) { out.push((n.nodeValue || '').replace(/\s+/g, ' ')); continue; }
        if (n.nodeType !== 1) continue;
        var tag = n.tagName;
        if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'BUTTON') continue;
        var block = /^(DIV|P|LI|UL|OL|TABLE|TR|H[1-6]|BLOCKQUOTE|PRE|BR)$/.test(tag);
        if (block) out.push('\n');
        walk(n, out);
        if (block) out.push('\n');
      }
    }
    var parts = [];
    walk(el, parts);
    // 行内块（span 相邻元素）之间保底一个空格；再压空白
    var s = parts.join(' ')
      .replace(/[ \t]*\n[ \t]*/g, '\n')
      .replace(/\n{3,}/g, '\n\n')
      .replace(/[ \t]{2,}/g, ' ')
      .trim();
    // 元素边界补的空格可能造成中英文间多余空格，但保留了可读性——压缩「空格+换行」交界
    return s.replace(/ ?\n ?/g, '\n');
  }

  function findTurns() {
    return qsa(document, '.ds-message');
  }

  function topicName() {
    // 会话名三级来源：侧栏高亮项 → document.title（官网会把会话名写入标题，
    // 2026-10-25 真机检验发现侧栏选择器不稳，兜底名「DeepSeek 对话」会顶掉真名）
    var t = '';
    try {
      var act = document.querySelector('[class*="conversation"][class*="active"],[class*="chat-item"][class*="active"]');
      if (act) t = nodeText(act).split('\n')[0];
    } catch (e) {}
    if (!t) {
      t = (document.title || '').replace(/- DeepSeek\b.*$/i, '').trim();
    }
    if (!t || t === 'DeepSeek' || t === 'Chat') {
      var h = document.querySelector('h1');
      if (h) t = nodeText(h).slice(0, 40);
    }
    t = (t || '').replace(/[\\/:*?"<>|\r\n]+/g, ' ').trim().slice(0, 24);
    return t || 'DeepSeek 对话';
  }

  // ---------- HTML → Markdown ----------
  function inlineText(node) {
    var out = '';
    for (var n = node.firstChild; n; n = n.nextSibling) {
      if (n.nodeType === 3) { out += n.nodeValue; continue; }
      if (n.nodeType !== 1) continue;
      var tag = n.tagName, inner = inlineText(n);
      if (tag === 'CODE') out += '`' + inner + '`';
      else if (tag === 'STRONG' || tag === 'B') out += '**' + inner + '**';
      else if (tag === 'EM' || tag === 'I') out += '*' + inner + '*';
      else if (tag === 'DEL' || tag === 'S') out += '~~' + inner + '~~';
      else if (tag === 'A') out += '[' + inner + '](' + (n.getAttribute('href') || '') + ')';
      else if (tag === 'BR') out += '\n';
      else out += inner;
    }
    return out;
  }
  function mdBlock(root) {
    var out = '';
    for (var n = root.firstChild; n; n = n.nextSibling) {
      if (n.nodeType === 3) { out += n.nodeValue || ''; continue; }
      if (n.nodeType !== 1) continue;
      var tag = n.tagName, inner = inlineText(n);
      if (/^H[1-6]$/.test(tag)) out += '\n\n' + '#'.repeat(+tag[1]) + ' ' + inner.trim() + '\n\n';
      else if (tag === 'P') out += '\n\n' + inner.trim() + '\n\n';
      else if (tag === 'UL' || tag === 'OL') {
        var idx = 1;
        for (var li = n.firstElementChild; li; li = li.nextElementSibling) {
          if (li.tagName !== 'LI') continue;
          out += (tag === 'OL' ? (idx++) + '. ' : '- ') + inlineText(li).trim() + '\n';
        }
        out += '\n';
      }
      else if (tag === 'PRE' || (tag === 'DIV' && n.querySelector('code'))) {
        var code = n.querySelector('code');
        var lang = (((code && code.className) || '').match(/language-([A-Za-z0-9_+-]+)/) || [])[1] || '';
        out += '\n\n```' + lang + '\n' + ((code || n).textContent || '').replace(/\n$/, '') + '\n```\n\n';
      }
      else if (tag === 'BLOCKQUOTE')
        out += '\n\n' + inlineText(n).trim().split('\n').map(function (l) { return '> ' + l; }).join('\n') + '\n\n';
      else if (tag === 'TABLE') {
        var rows = qsa(n, 'tr');
        for (var r = 0; r < rows.length; r++) {
          var cells = qsa(rows[r], 'th,td'), line = '|';
          for (var c = 0; c < cells.length; c++) line += ' ' + inlineText(cells[c]).trim() + ' |';
          out += line + '\n';
          if (r === 0 && cells.length) out += '|' + ' --- |'.repeat(cells.length) + '\n';
        }
        out += '\n';
      }
      else if (tag === 'HR') out += '\n\n---\n\n';
      else if (tag === 'BUTTON' || tag === 'SCRIPT' || tag === 'STYLE') continue;
      else out += mdBlock(n);
    }
    return out;
  }
  function squash(s) {
    return s.replace(/\n{3,}/g, '\n\n').replace(/[ \t]+\n/g, '\n').trim();
  }

  // ---------- 收集：Markdown 与纯文本双份 ----------
  // 官网消息区是虚拟滚动：DOM 只挂视口附近的消息，滚到中段导出会丢前后的轮次
  // （实测同一会话导出 14 轮/8 轮不等）。抓取前先滚到顶，等历史全量挂载、
  // DOM 稳定后再抓，抓完滚回原位。
  // 异步收集：官网消息区是 virtua 虚拟列表（ds-virtual-list--printable），DOM 只挂
  // 视口附近的消息；它不是原生滚动（scrollHeight 恒定、scrollTop 恒 0，内部用
  // transform 移动），逐屏滚动方案无效（实测）。--printable 是 virtua 的打印模式
  // 标记：派发 beforeprint 事件会全量挂载所有条目，afterprint 恢复虚拟化——
  // 这正是导出需要的官方通道。流程：beforeprint → 等挂载稳定 → 抓取 → afterprint。
  function collectAsync(cb) {
    var stableRounds = 0, lastCount = -1, attempts = 0;
    function dispatch(name) {
      try { document.dispatchEvent(new Event(name)); } catch (e) {}
      try { window.dispatchEvent(new Event(name)); } catch (e) {}
    }
    dispatch('beforeprint');
    function step() {
      var count = document.querySelectorAll('.ds-message').length;
      if (count === lastCount) stableRounds++;
      else { stableRounds = 0; lastCount = count; }
      // 挂载稳定（连续 5 帧 = ~80ms 不变）或 3s 兜底
      if (stableRounds >= 5 || attempts > 180) {
        var data = collectNow();
        dispatch('afterprint');
        cb(data);
        return;
      }
      attempts++;
      requestAnimationFrame(step);
    }
    requestAnimationFrame(step);
  }

  function collectNow() {
    return collectFromNodes(findTurns());
  }

  // 从给定消息节点列表（可能来自跨滚动位置的并集池）构建导出数据。
  // 注意：池中节点可能已被虚拟列表卸载出文档，textContent 仍可读
  // （节点对象保持引用即保留子树），但 querySelector 只对仍在文档里的
  // 节点有效——所以必须在 harvest 时、节点还挂载的那一刻提取文本。
  function collectFromNodes(msgs) {
    var turns = [], texts = [];
    for (var i = 0; i < msgs.length; i++) {
      var m = msgs[i];
      // 回答正文只认 main-content（思考容器内也有 .ds-markdown，fallback 会把
      // 思考内容误当回答导出，见 2026-10-25 真机检验）；无标记则视为无正文
      var md = m.querySelector('.ds-markdown.ds-assistant-message-main-content');
      if (md) {
        // 助手轮：思考块（可选，需在正文之外查找避免自包含）+ 回答正文
        var think = null;
        var mds = m.querySelectorAll('.ds-markdown.ds-assistant-message-main-content');
        var thinks = m.querySelectorAll('.ds-think-content');
        for (var t = 0; t < thinks.length; t++) {
          if (!mds.length || !thinks[t].contains(mds[0])) { think = thinks[t]; break; }
        }
        var aText = squash(mdBlock(md));
        if (!aText && !think) continue;
        turns.push({
          u: '',
          a: aText,
          think: think ? squash(mdBlock(think)) : ''
        });
        texts.push({ u: '', a: nodeText(md), think: think ? nodeText(think) : '' });
      } else {
        // 用户轮：纯文本气泡；附件轮（含 eafda4ae 附件容器，2026-10-25 探针实测）
        // 格式化为「用户文字 + 每附件一行 📎 文件名 类型 大小」。
        // 实现：克隆消息节点并移除附件容器后取正文（附件文本不再混入），
        // 附件行从容器文本单独提取——彻底避免原始串与 📎 行重复
        var uText = '';
        var lines = [];
        try {
          var SEL = '.eafda4ae, [class*="file-card"], [class*="attachment"]';
          var fboxes = m.querySelectorAll(SEL);
          var clone = m.cloneNode(true);
          var cboxes = clone.querySelectorAll(SEL);
          for (var ci = cboxes.length - 1; ci >= 0; ci--) cboxes[ci].remove();
          uText = nodeText(clone);
          for (var fi = 0; fi < fboxes.length; fi++) {
            var box = fboxes[fi];
            // 父容器（内部还嵌着附件节点）跳过，只取叶子，避免整组重复
            if (box.querySelector(SEL)) continue;
            var ft = nodeText(box).replace(/\n+/g, ' ').trim();
            if (!ft) continue;
            // 一个容器可能装多个附件（如 Icon.txt/ppt/html 成组上传）：
            // 全局拆「名字 类型 大小」三元组，每个附件独立一行。
            // 类型标签须为大写独立词（正则无 i 标志，文件名里的小写 .txt 不会误配）
            var re = /(\S+)\s+(TXT|YML|JSON|MD|PDF|PNG|JPG|JPEG|GIF|ZIP|RAR|7Z|DOC|DOCX|XLS|XLSX|PPT|PPTX|CSV|PY|JS|TS|CPP|C|JAVA|HTML|CSS|XML|LOG|INK|APK|EXE)\s+([\d.]+\s*[KMG]?B)/g;
            var matches = ft.match(re);
            if (matches && matches.length) {
              matches.forEach(function (x) { lines.push('📎 ' + x.trim()); });
            } else {
              lines.push('📎 ' + ft);
            }
          }
          if (lines.length && uText) {
            uText = uText + '\n' + lines.join('\n');
          } else if (lines.length) {
            uText = lines.join('\n');
          }
        } catch (e) {
          if (!uText) uText = nodeText(m);
        }
        if (!uText) continue;
        turns.push({ u: uText, a: '', think: '' });
        texts.push({ u: uText, a: '', think: '' });
      }
    }
    if (turns.length === 0) return null;
    return { title: topicName(), turns: turns, turnsText: texts };
  }

  function turnsToMarkdown(turns) {
    return turns.map(function (t) {
      var s = '';
      if (t.u) s += '### 🧑 用户\n\n' + t.u + '\n\n';
      if (t.think) s += '> 💭 **思考过程**\n>\n'
        + t.think.split('\n').map(function (l) { return '> ' + l; }).join('\n') + '\n\n';
      if (t.a) s += '### 🤖 DeepSeek\n\n' + t.a;
      return s.trim();
    }).filter(function (s) { return s; }).join('\n\n---\n\n');
  }
  function turnsToText(turns) {
    return turns.map(function (t) {
      var s = '';
      if (t.u) s += '【用户】\n' + t.u + '\n\n';
      if (t.think) s += '【思考】\n' + t.think + '\n\n';
      if (t.a) s += '【DeepSeek】\n' + t.a;
      return s.trim();
    }).filter(function (s) { return s; }).join('\n\n' + '='.repeat(40) + '\n\n');
  }
  function turnsToJson(turns) {
    return JSON.stringify({
      app: 'DeepSeekChat', version: '0.1.0', title: topicName(),
      exported_at: new Date().toISOString(),
      turns: turns.map(function (t, i) {
        var o = { index: i + 1 };
        if (t.u) o.user = t.u;
        if (t.think) o.thinking = t.think;
        if (t.a) o.assistant = t.a;
        return o;
      })
    }, null, 2);
  }
  function turnsToHtml(turns) {
    var body = turns.map(function (t) {
      var h = '';
      if (t.u) h += '<div class="user">' + esc(t.u) + '</div>';
      if (t.think) h += '<div class="think">' + esc(t.think) + '</div>';
      if (t.a) h += '<div class="assistant">' + mdToHtml(t.a) + '</div>';
      return '<div class="turn">' + h + '</div>';
    }).join('\n');
    return '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">'
      + '<title>' + esc(topicName()) + '</title><style>'
      + 'body{font:15px/1.7 system-ui,sans-serif;max-width:800px;margin:32px auto;padding:0 16px;color:#1b1f27}'
      + '.turn{margin:24px 0;padding:16px;border-radius:12px;background:#f7f8fa}'
      + '.user{font-weight:600;margin-bottom:10px;white-space:pre-wrap}'
      + '.think{border-left:3px solid #c8cdd8;padding:6px 12px;margin:8px 0;color:#5a6070;font-size:13px;white-space:pre-wrap}'
      + '.assistant{border-top:1px solid #e8eaef;padding-top:10px}'
      + '.assistant pre{background:#1b1f27;color:#e8eaf0;padding:12px;border-radius:8px;overflow:auto}'
      + '.assistant code{font-family:Consolas,monospace;font-size:13px}'
      + '</style></head><body><h1>' + esc(topicName()) + '</h1>' + body + '</body></html>';
  }
  function esc(s) {
    return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
  function mdToHtml(md) {
    // 轻量：段落 + 代码块 + 行内样式，导出可读即可
    var h = esc(md);
    h = h.replace(/```([A-Za-z0-9_+-]*)\n([\s\S]*?)```/g, function (m, l, c) {
      return '<pre><code>' + c + '</code></pre>';
    });
    h = h.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
         .replace(/\*([^*]+)\*/g, '<em>$1</em>')
         .replace(/`([^`]+)`/g, '<code>$1</code>');
    return h;
  }

  // ---------- 导出入口 ----------
  var FORMATS = [
    { id: 'md',   label: 'Markdown (.md)' },
    { id: 'txt',  label: '纯文本 (.txt)' },
    { id: 'html', label: '网页 (.html)' },
    { id: 'doc',  label: 'Word 文档 (.doc)' },
    { id: 'json', label: 'JSON 数据 (.json)' }
  ];

  function runExport(fmt) {
    // 异步路径：先滚到顶等全量挂载（虚拟滚动修复），抓完自动回滚原位
    collectAsync(function (data) {
      if (!data) return;
      var payload = { format: fmt, title: data.title };
      if (fmt === 'md') payload.markdown = turnsToMarkdown(data.turns);
      else if (fmt === 'txt') payload.text = turnsToText(data.turnsText);
      else if (fmt === 'html' || fmt === 'doc') payload.html = turnsToHtml(data.turns);
      else if (fmt === 'json') payload.json = turnsToJson(data.turns);
      emit('dschat://export', payload);
    });
  }

  // ---------- UI 挂载 ----------
  var ICON_DL = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>';
  // 刷新图标：Edge 风格——整圆弧 + 顶部顺时针箭头（用户指定参考）
  var ICON_RL = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
    + '<path d="M21 12a9 9 0 1 1-2.64-6.36"/>'
    + '<path d="M21 3v6h-6"/>'
    + '</svg>';

  // ---------- 可见性（FR-22）：登录页 / 新会话欢迎页不显示工具条 ----------
  // 判据与抓取共用：存在 .ds-message（任一消息）= 有对话。260ms 弹性出入场动画。
  var visTimer = 0;
  var lastVis = null;
  function applyVisibility(show) {
    var bar = document.getElementById('dschat-bar');
    if (!bar) return;
    if (show === lastVis) return;
    lastVis = show;
    if (show) {
      bar.style.display = '';
      requestAnimationFrame(function () { bar.classList.remove('dschat-hidden'); });
    } else {
      bar.classList.add('dschat-hidden');
      var menu = document.getElementById('dschat-export-menu');
      if (menu) menu.classList.remove('open');
      window.setTimeout(function () {
        if (lastVis === false) bar.style.display = 'none';
      }, 280);
    }
  }
  function scheduleVisibility() {
    if (visTimer) return;
    visTimer = window.setTimeout(function () {
      visTimer = 0;
      applyVisibility(!!document.querySelector('.ds-message'));
    }, 120);
  }

  function mount() {
    ensureStyle();
    if (document.getElementById('dschat-bar')) return;
    if (!document.body) return;

    var bar = document.createElement('div');
    bar.id = 'dschat-bar';

    var reload = document.createElement('button');
    reload.className = 'dschat-xbtn'; reload.type = 'button';
    reload.title = '刷新页面（F5 / Ctrl+R）';
    reload.setAttribute('aria-label', '刷新页面');
    reload.innerHTML = ICON_RL;
    reload.addEventListener('click', function () { emit('dschat://reload', {}); });

    var dl = document.createElement('button');
    dl.className = 'dschat-xbtn'; dl.type = 'button';
    dl.title = '导出对话';
    dl.setAttribute('aria-label', '导出对话');
    dl.innerHTML = ICON_DL;

    var menu = document.createElement('div');
    menu.id = 'dschat-export-menu';
    FORMATS.forEach(function (f) {
      var b = document.createElement('button');
      b.className = 'dschat-mi'; b.type = 'button';
      b.textContent = f.label;
      b.addEventListener('click', function () {
        menu.classList.remove('open');
        runExport(f.id);
      });
      menu.appendChild(b);
    });

    dl.addEventListener('click', function (e) {
      e.stopPropagation();
      menu.classList.toggle('open');
    });
    document.addEventListener('click', function (e) {
      if (menu.classList.contains('open')
        && !menu.contains(e.target) && e.target !== dl && !dl.contains(e.target)) {
        menu.classList.remove('open');
      }
    });

    bar.appendChild(menu);   // 菜单是 bar 子元素：CSS 变量经继承生效（深浅跟随的前提）
    bar.appendChild(reload);
    bar.appendChild(dl);
    document.body.appendChild(bar);
    bar.classList.add('dschat-hidden');
    lastVis = null;
    // 初值优先级：标题栏事件结论 > 本页自测（新文档早期 html/body 常为默认白，
    // 自测会误判浅色——刷新后按钮回白的根因）
    if (pendingDark !== null) setDark(pendingDark); else applyTheme();
    scheduleVisibility();
    // 消息区随 SPA 渲染变化：观察驱动可见性；主题由 html class 观察者单独负责
    try {
      new MutationObserver(scheduleVisibility).observe(document.body, {
        childList: true, subtree: true
      });
    } catch (e) {}
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', mount);
  } else {
    mount();
  }
  setTimeout(mount, 2500);
})();
"#;

/// debug 探针脚本（DSCHAT_EXPORT_PROBE=1）：导出官网真实 DOM 结构——
/// 全页 class 频率、markdown 容器父链、侧栏与消息区 HTML 片段，分三条上报。
#[cfg(debug_assertions)]
pub const EXPORT_PROBE_JS: &str = r#"
(function () {
  function emit(tag, s) {
    try {
      window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {
        event: 'dschat://log', payload: { msg: 'EXPORT_PROBE@@' + tag + '|' + s }
      });
    } catch (e) {}
  }
  function waitBody(cb) {
    var t0 = Date.now();
    (function check() {
      if (document.body && document.querySelectorAll('body *').length > 30) return cb();
      if (Date.now() - t0 > 45000) return cb();
      setTimeout(check, 800);
    })();
  }

  waitBody(function () {
    // 点开侧栏第一个历史会话（_546d736 疑似列表项类；多选择器兜底），等渲染后采样
    function clickSide(fn) {
      var sels = ['._546d736', '[class*="conversation-item"]', '[class*="chat-list"] li'];
      for (var i = 0; i < sels.length; i++) {
        var items = document.querySelectorAll(sels[i]);
        for (var j = 0; j < items.length; j++) {
          var t = (items[j].textContent || '').trim();
          if (t.length > 4 && t.length < 60) {
            items[j].click();
            fn(4000);
            return;
          }
        }
      }
      fn(800);
    }
    clickSide(function (ms) {
    // 1) 全页 class 频率 top40
    var counts = {};
    try {
      var els = document.querySelectorAll('body *');
      for (var i = 0; i < els.length && i < 4000; i++) {
        var c = els[i].className;
        if (typeof c !== 'string' || !c) continue;
        var seen = {};
        c.split(/\s+/).forEach(function (k) {
          if (k && !seen[k]) { seen[k] = 1; counts[k] = (counts[k] || 0) + 1; }
        });
      }
    } catch (e) {}
    emit('FREQ', JSON.stringify(
      Object.keys(counts).sort(function (a, b) { return counts[b] - counts[a]; })
        .slice(0, 40).map(function (k) { return k + ':' + counts[k]; })
    ));

    // 2) markdown 容器父链（定位轮容器结构）
    try {
      var md = document.querySelector('[class*="markdown"]');
      var chain = [];
      for (var p = md; p && p !== document.body && chain.length < 8; p = p.parentElement) {
        chain.push(p.tagName + '[' + String(p.className).slice(0, 100) + ']');
      }
      emit('MDCHAIN', JSON.stringify(chain));
    } catch (e) { emit('MDCHAIN', 'ERR'); }

    // 3) 侧栏 + 消息区结构（点开会话后执行）
    setTimeout(function () {
      try {
        var side = document.querySelector('aside, nav, [class*="sidebar"], [class*="side-bar"]');
        emit('SIDE', side ? side.outerHTML.replace(/\s+/g, ' ').slice(0, 1500) : 'NULL');
      } catch (e) { emit('SIDE', 'ERR'); }
      try {
        // 对话流容器：含 >=2 个 .ds-markdown 的最小祖先
        var mds = document.querySelectorAll('.ds-markdown');
        var flow = null;
        if (mds.length >= 2) {
          var p = mds[0].parentElement;
          while (p && p !== document.body) {
            if (p.querySelectorAll('.ds-markdown').length >= 2) { flow = p; break; }
            p = p.parentElement;
          }
        }
        if (flow) {
          var kids = [];
          for (var k = flow.firstElementChild; k; k = k.nextElementSibling) {
            var info = k.tagName + '[' + String(k.className).slice(0, 70) + ']';
            info += ' md=' + k.querySelectorAll('.ds-markdown').length;
            info += ' think=' + k.querySelectorAll('.ds-think-content').length;
            info += ' txt=' + (k.textContent || '').trim().slice(0, 24);
            kids.push(info);
          }
          emit('KIDS', JSON.stringify({ flow: String(flow.className).slice(0, 90), n: kids.length, kids: kids.slice(0, 10) }));
          emit('HEAD', flow.outerHTML.replace(/\s+/g, ' ').slice(0, 2000));
        } else {
          emit('KIDS', 'NO_FLOW md=' + mds.length);
        }
      } catch (e) { emit('KIDS', 'ERR ' + e); }
      // 4) .ds-message 全集：类名 + 是否含 markdown/think + 文本开头（确认用户轮结构）
      try {
        var msgs = document.querySelectorAll('.ds-message');
        var rows = [];
        for (var i = 0; i < msgs.length && i < 8; i++) {
          rows.push({
            cls: String(msgs[i].className).slice(0, 60),
            md: msgs[i].querySelectorAll('.ds-markdown').length,
            think: msgs[i].querySelectorAll('.ds-think-content').length,
            txt: (msgs[i].textContent || '').trim().slice(0, 20)
          });
        }
        emit('MSGS', JSON.stringify({ n: msgs.length, rows: rows }));
      } catch (e) { emit('MSGS', 'ERR'); }

      // 5) 附件消息结构探针：找含「TXT|YML|KB」特征的气泡，dump 其子元素结构
      setTimeout(function () {
        try {
          var hits = [];
          var all = document.querySelectorAll('.ds-message, .d29f3d7d');
          for (var i = 0; i < all.length && hits.length < 3; i++) {
            var t = all[i].textContent || '';
            if (!/KB/.test(t) || !/\.(txt|yml|json|md|pdf|png|zip|doc)/i.test(t)) continue;
            var kids = [];
            for (var k = all[i].firstElementChild; k && kids.length < 8; k = k.nextElementSibling) {
              kids.push(k.tagName + '.' + String(k.className).slice(0, 40) + ':「' + (k.textContent || '').trim().slice(0, 40) + '」');
            }
            hits.push({ cls: String(all[i].className).slice(0, 50), kids: kids });
          }
          emit('ATTACH', JSON.stringify(hits));
        } catch (e) { emit('ATTACH', 'ERR ' + e); }
      }, 4000);
    }, 2500);
    });
  });
})();
"#;

const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;

/// 本进程导出序号（文件名唯一化，消除同秒 TOCTOU 覆盖竞态）
static EXPORT_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// 文件名清洗：Windows 非法字符与控制符 → 空格，压缩空白，限 24 字符；空则回退默认名。
fn sanitize_title(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\r' | '\n' => ' ',
            c if (c as u32) < 0x20 => ' ',
            c => c,
        })
        .collect::<String>();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // 限 24 字符：保证「会话名-时间」整体在资源管理器里通常不被截断
    let out: String = joined.chars().take(24).collect();
    let out = out.trim_end();
    if out.is_empty() { None } else { Some(out.to_string()) }
}

/// 文件名时间戳（「会话名+时间」，用户指定模板 AAAA-AA-AA_AA:AA:AA）：
/// Windows 文件名禁止冒号，时间分隔以 `-` 呈现：2026-10-25_16-35-07。
pub fn timestamp_compact() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let st = unsafe { GetLocalTime() };
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

/// 导出文件：data\Export session\<标题>-<时间>.<ext>（「会话名+时间」），重名加序号。
fn save_bytes(title: &str, ext: &str, body: &[u8]) -> Result<(String, std::path::PathBuf), String> {
    if body.len() > MAX_EXPORT_BYTES {
        return Err(format!("导出内容过大（{} KB）", body.len() / 1024));
    }
    let title = sanitize_title(title).ok_or_else(|| "导出标题为空".to_string())?;
    let dir = crate::paths::data_dir().join("Export session");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建导出目录失败: {e}"))?;

    let stamp = timestamp_compact();
    let seq = EXPORT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // 序号后缀消除同秒并发导出的 check-then-act 竞态（两个窗口/连续两次导出
    // 同一标题时 exists() 检查与实际写入之间存在窗口，后写会覆盖先写）
    let stamp = if seq == 0 { stamp } else { format!("{stamp}-{seq}") };
    let mut path = dir.join(format!("{title}_{stamp}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{title}_{stamp}({n}).{ext}"));
        n += 1;
    }
    atomic_write(&path, body)?;

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok((name, dir))
}

/// 导出事件处理（FR-22）：页面载荷直接落盘。
pub fn handle_export_event(app: &tauri::AppHandle, payload: &Value) {
    use tauri_plugin_opener::OpenerExt as _;

    let format = payload.get("format").and_then(|v| v.as_str()).unwrap_or("md");
    let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("");

    let (ext, body): (&str, Vec<u8>) = match format {
        "md" => (
            "md",
            payload
                .get("markdown")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
        ),
        "txt" => (
            "txt",
            payload.get("text").and_then(|v| v.as_str()).unwrap_or("").as_bytes().to_vec(),
        ),
        "json" => (
            "json",
            payload.get("json").and_then(|v| v.as_str()).unwrap_or("").as_bytes().to_vec(),
        ),
        "html" | "doc" => {
            // Word（.doc）= Word 可直接打开的 HTML 单文件。原位注入：给 <html> 加
            // Office 命名空间、<meta> 后插 WordDocument 声明——不重组标签顺序
            // （此前重拼会把 <title> 挤到 </head> 之后，真机检验发现后改此法）
            let html = payload.get("html").and_then(|v| v.as_str()).unwrap_or("");
            let bytes = if format == "doc" {
                let mut out = html.replacen(
                    "<html lang=\"zh-CN\">",
                    "<html lang=\"zh-CN\" xmlns:w=\"urn:schemas-microsoft-com:office:word\">",
                    1,
                );
                if let Some(pos) = out.find("<meta charset=\"utf-8\">") {
                    let ins = pos + "<meta charset=\"utf-8\">".len();
                    out.insert_str(
                        ins,
                        "<!--[if gte mso 9]><xml><w:WordDocument><w:View>Print</w:View>\
                         </w:WordDocument></xml><![endif]-->",
                    );
                }
                out.into_bytes()
            } else {
                html.as_bytes().to_vec()
            };
            (if format == "doc" { "doc" } else { "html" }, bytes)
        }
        _ => return,
    };

    // 空载荷防护：内容字段缺失/非字符串/为空时不落盘（空文件以成功姿态
    // 打开目录是对用户的欺骗），失败只记日志——导出无 UI 反馈（用户指定），
    // 成功路径由「自动打开导出目录」天然反馈
    if body.iter().all(|&b| b == b' ' || b == b'\n' || b == b'\r' || b == b'\t') {
        log::warn!("导出载荷正文为空（format={format}），已拒绝落盘");
        return;
    }

    match save_bytes(title, ext, &body) {
        Ok((name, dir)) => {
            log::info!("对话已导出: {name}");
            if let Err(e) = app.opener().open_path(dir.display().to_string(), None::<&str>) {
                log::warn!("打开导出目录失败: {e}");
            }
        }
        Err(e) => log::warn!("对话导出失败: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_illegal_and_bounds_length() {
        assert_eq!(sanitize_title("a/b\\c:d*e?f\"g<h>i|j").unwrap(), "a b c d e f g h i j");
        assert_eq!(sanitize_title("  多 空格  ").unwrap(), "多 空格");
        assert_eq!(sanitize_title(""), None);
        assert_eq!(sanitize_title("///"), None);
        let long = sanitize_title(&"长".repeat(100)).unwrap();
        assert!(long.chars().count() <= 24);
    }
}
