import http from "node:http";

const PORT = Number(process.env.PORT ?? 8765);
const TEXT = "这是 LingoNote 的模拟翻译。\n第二行用于测试换行，以及流式输出。";
const CHUNKS = ["这是 ", "LingoNote ", "的模拟翻译。\n", "第二行用于测试换行，", "以及流式输出。"];

function json(res, status, value) {
  const body = JSON.stringify(value);
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(body),
    "Connection": "close",
  });
  res.end(body);
}

const server = http.createServer((req, res) => {
  if (req.method === "GET" && req.url === "/health") {
    json(res, 200, { ok: true, service: "lingonote-mock" });
    return;
  }

  if (req.method !== "POST") {
    json(res, 404, { error: "not found" });
    return;
  }

  let body = "";
  req.setEncoding("utf8");
  req.on("data", (chunk) => {
    body += chunk;
  });
  req.on("end", () => {
    let payload = {};
    try {
      payload = JSON.parse(body);
    } catch {
      // 忽略非法 JSON，按默认流式处理。
    }

    if (!payload.stream) {
      json(res, 200, {
        id: "mock",
        object: "chat.completion",
        choices: [
          {
            index: 0,
            message: { role: "assistant", content: TEXT },
            finish_reason: "stop",
          },
        ],
      });
      return;
    }

    res.writeHead(200, {
      "Content-Type": "text/event-stream; charset=utf-8",
      "Cache-Control": "no-cache",
      "Connection": "close",
    });

    let index = 0;
    const timer = setInterval(() => {
      if (index < CHUNKS.length) {
        const data = {
          id: "mock",
          object: "chat.completion.chunk",
          choices: [{ index: 0, delta: { content: CHUNKS[index] }, finish_reason: null }],
        };
        res.write(`data: ${JSON.stringify(data)}\n\n`);
        index += 1;
      } else {
        res.write("data: [DONE]\n\n");
        clearInterval(timer);
        res.end();
      }
    }, 120);
  });
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`LingoNote mock server: http://127.0.0.1:${PORT}/v1`);
  console.log("Base URL: http://127.0.0.1:8765/v1");
  console.log("API Key: mock");
  console.log("Model: mock");
});
