// Drives the running app's WebView2 over the DevTools protocol, for checking the UI from a script.
// Start the app with: WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 pnpm tauri dev
//
//   node scripts/webview.mjs shot <out.png>        screenshot of the window's page
//   node scripts/webview.mjs eval "<js expression>" evaluate in the page, print the JSON result
//   node scripts/webview.mjs click "<button text>"  click the first button whose text contains this
import { writeFileSync } from "node:fs";

const port = process.env.WEBVIEW_PORT ?? "9333";
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) throw new Error("no page target; is the app running with remote debugging?");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});
let id = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  }
};
const send = (method, params = {}) =>
  new Promise((resolve) => {
    pending.set(++id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });
const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};

const [command, arg] = process.argv.slice(2);
if (command === "shot") {
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(arg, Buffer.from(r.result.data, "base64"));
  console.log(arg);
} else if (command === "eval") {
  console.log(JSON.stringify(await evaluate(arg), null, 1));
} else if (command === "click") {
  const clicked = await evaluate(`(() => {
    const b = [...document.querySelectorAll("button")].find((b) => b.innerText.includes(${JSON.stringify(arg)}));
    if (!b) return false;
    b.scrollIntoView({ block: "center" });
    b.click();
    return b.innerText;
  })()`);
  console.log(clicked === false ? `no button with "${arg}"` : `clicked: ${clicked}`);
}
ws.close();
