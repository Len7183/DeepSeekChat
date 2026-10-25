// check-release.mjs 路径正则的独立验证（node scripts/test-check-release.cjs）
const reSrc = String.raw`[A-Za-z]:[\\/][^\n\r"'` + '`' + String.raw`\\/:\*\?<>|]{1,60}(?:[\\/][^\n\r"'` + '`' + String.raw`\\/:\*\?<>|]{1,60}){1,}`;
const re = new RegExp(reSrc);
const cases = [
  [String.raw`C:\Users\张三\AppData\Roaming`, true],
  [String.raw`C:/Users/李四/data/config.json`, true],
  [String.raw`C:\build\target`, true],
  ["plain text without paths 张三", false],
  ["version 0.1.0 release", false],
  [String.raw`D:\Program Files\DeepSeekChat\data`, true],
];
let ok = true;
for (const [s, want] of cases) {
  const got = re.test(s);
  const status = got === want ? "PASS" : "FAIL";
  if (got !== want) ok = false;
  console.log(status, JSON.stringify(s), "->", got);
}
console.log(ok ? "ALL PASS" : "SOME FAILED");
process.exitCode = ok ? 0 : 1;
