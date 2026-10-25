import { defineConfig } from "vite";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// DeepSeekChat 前端：设置窗口 / 错误页 / 托盘菜单，均为无框架原生 TS 页面。
// 主窗口直接加载 https://chat.deepseek.com，不经由此处。
// package.json 为 "type": "module"：ESM 下 __dirname 不存在（Vite 7 已不再
// 支持 CJS 配置），用 fileURLToPath + import.meta.url 派生（Node ≥20.11
// 也可直接用 import.meta.dirname，这里取兼容面更宽的写法）
const here = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "chrome120",
    rollupOptions: {
      input: {
        settings: resolve(here, "settings.html"),
        error: resolve(here, "error.html"),
        menu: resolve(here, "menu.html"),
      },
    },
  },
});
