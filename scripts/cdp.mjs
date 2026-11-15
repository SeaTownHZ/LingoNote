// 开发期 UI 烟测工具：通过 WebView2 的 CDP 端口执行 JS 或截图。
// 用法：
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222"
//   node scripts/cdp.mjs list
//   node scripts/cdp.mjs eval "document.title"
//   node scripts/cdp.mjs screenshot docs/images/ui.png
//
// 可用环境变量：CDP_PORT（默认 9222）、CDP_TARGET（页面 URL 子串，默认 settings.html）。

import { writeFile } from "node:fs/promises";

const args = process.argv.slice(2);
const command = args[0] ?? "list";
const port = process.env.CDP_PORT ?? "9222";
const targetHint = process.env.CDP_TARGET ?? "settings.html";

let targets;
try {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`);
  targets = await response.json();
} catch (error) {
  console.error(`无法连接 CDP 端口 ${port}：${error.message}`);
  console.error(
    "请先以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 启动应用。",
  );
  process.exit(1);
}

if (command === "list") {
  console.log(
    JSON.stringify(
      targets.map((target) => ({
        type: target.type,
        title: target.title,
        url: target.url,
      })),
      null,
      2,
    ),
  );
  process.exit(0);
}

const target =
  targets.find((item) => item.type === "page" && item.url.includes(targetHint)) ??
  targets.find((item) => item.type === "page");

if (!target || !target.webSocketDebuggerUrl) {
  console.error(`找不到匹配 ${targetHint} 的页面`);
  process.exit(1);
}

const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = () => reject(new Error("CDP WebSocket 连接失败"));
});

let nextId = 0;
const pending = new Map();

socket.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (!message.id || !pending.has(message.id)) return;
  const { resolve, reject } = pending.get(message.id);
  pending.delete(message.id);
  if (message.error) reject(new Error(message.error.message));
  else resolve(message.result);
};

function send(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
}

if (command === "eval") {
  const expression = args.slice(1).join(" ");
  const result = await send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
    userGesture: true,
  });
  if (result.exceptionDetails) {
    console.error(JSON.stringify(result.exceptionDetails, null, 2));
    socket.close();
    process.exit(1);
  }
  const value = result.result?.value;
  console.log(typeof value === "string" ? value : JSON.stringify(value, null, 2));
} else if (command === "screenshot") {
  const path = args[1] ?? "cdp-screenshot.png";
  await send("Page.enable");
  const shot = await send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  await writeFile(path, Buffer.from(shot.data, "base64"));
  console.log(path);
} else {
  console.error(`未知命令：${command}（可用：list / eval / screenshot）`);
  socket.close();
  process.exit(1);
}

socket.close();
process.exit(0);
