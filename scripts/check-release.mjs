import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// Check the uncompressed application EXE, not the compressed NSIS container.
assert.ok(process.argv.length > 2, "Pass at least one application EXE path");
for (const path of process.argv.slice(2)) {
  const bytes = readFileSync(path);
  // 三种解码：latin1（逐字节）、utf16le（偶偏移）、utf16le（奇偏移）。
  // 中文用户名的本机路径（C:\Users\张三\...）只在 UTF-8/UTF-16 解码下可读，
  // latin1 字节视角会漏检——三种全查才不会放过非 ASCII 路径组件。
  const texts = [
    ["latin1", bytes.toString("latin1")],
    ["utf16le", bytes.toString("utf16le")],
    ["utf16le+1", bytes.subarray(1).toString("utf16le")],
  ];
  for (const [encoding, text] of texts) {
    // 盘符路径检测。三重防 URL 误报：
    // 1) 盘符字母前不能是 scheme 字符（https: 的 s 前面是 p，
    //    用「非 scheme 字符开头 + 捕获组」实现）
    // 2) 冒号后不能紧跟 //（https://xxx 的形态，负向预查）
    // 组件允许非 ASCII（中文用户名），但要求可打印文本段——latin1 视角的
    // 压缩二进制噪声几乎不可能凑出连续可打印段。
    const re = new RegExp(
      "(?:^|[^A-Za-z0-9+.-])([A-Za-z]):[\\\\/](?!\\\\/)[\\x20-\\x7e\\u00a0-\\uffff]{2,60}(?:[\\\\/][\\x20-\\x7e\\u00a0-\\uffff]{2,60}){1,}",
      "g"
    );
    let m;
    while ((m = re.exec(text)) !== null) {
      // m[1] 是盘符字母；路径 = 盘符 + m[0] 中该字母之后的部分
      const full = m[0];
      const drive = m[1];
      const after = full.slice(full.indexOf(drive, full.length - drive.length - full.length) + 0);
      // 简化：直接从 full 中定位「盘符:」
      const driveIdx = full.lastIndexOf(drive + ":");
      const sPath = full.slice(driveIdx);
      const printable = sPath.replace(/[\\/]/g, "");
      if (printable.length >= 8 && !/[\x00-\x1f]/.test(sPath)) {
        assert.fail(`${path}: local absolute path embedded [${encoding}]: ${JSON.stringify(sPath.slice(0, 100))}`);
      }
    }
    assert.ok(
      !/DSCHAT_(?:CMD_TEST|DIALOG_PROBE|PROBE|EXPORT_PROBE|EXPORT_TEST)|CMD_TEST@@|DIALOG_PROBE@@|EXPORT_PROBE@@|--(?:menu-test|error-test|ext-test|settings)\b/.test(text),
      `${path}: debug entry embedded [${encoding}]`
    );
    assert.ok(
      !/-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----|\bgh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|\bAKIA[0-9A-Z]{16}|\bxox[baprs]-[A-Za-z0-9-]{10,}|\bAIza[0-9A-Za-z_-]{35}|\bsk-ant-[A-Za-z0-9_-]{20,}/.test(text),
      `${path}: credential signature embedded [${encoding}]`
    );
  }
  console.log(`Release privacy check passed: ${path}`);
}
