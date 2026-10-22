// No npm dependencies. Uses an installed Chromium/Edge over an anonymous CDP pipe.
import { spawn } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const executable = process.env.FASTIUM_TEST_BROWSER || [
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
].find(existsSync);
assert(executable, 'Set FASTIUM_TEST_BROWSER to a Chromium browser executable');
const out = resolve(root, '.tools/ui-test');
mkdirSync(out, { recursive: true });
const processBrowser = spawn(executable, ['--headless=new', '--no-first-run', '--no-default-browser-check',
  ...(process.env.FASTIUM_TEST_SOFTWARE === '1' ? ['--disable-gpu', '--in-process-gpu'] : []),
  '--disable-background-networking', '--remote-debugging-pipe', `--user-data-dir=${resolve(out, 'profile')}`, 'about:blank'],
  { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
let next = 1, buffer = '', errors = '', session;
const pending = new Map();
processBrowser.stderr.on('data', chunk => { errors = (errors + chunk).slice(-4000); });
processBrowser.stdio[4].on('data', chunk => {
  buffer += chunk.toString();
  let end;
  while ((end = buffer.indexOf('\0')) !== -1) {
    const message = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1);
    const request = pending.get(message.id);
    if (request) { pending.delete(message.id); clearTimeout(request.timer); message.error ? request.reject(new Error(JSON.stringify(message.error))) : request.resolve(message.result); }
  }
});
function call(method, params = {}, sessionId = session) {
  return new Promise((resolve, reject) => {
    const id = next++;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}\n${errors}`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    processBrowser.stdio[3].write(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }) + '\0');
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
try {
  const { targetId } = await call('Target.createTarget', { url: 'about:blank' });
  session = (await call('Target.attachToTarget', { targetId, flatten: true })).sessionId;
  await call('Page.enable');
  await call('Emulation.setDeviceMetricsOverride', { width: 1180, height: 780, deviceScaleFactor: 1, mobile: false });
  const { frameTree } = await call('Page.getFrameTree');
  const html = readFileSync(resolve(root, 'assets/shell.html'), 'utf8')
    .replace('<script>', '<script>window.messages=[];window.ipc={postMessage:s=>window.messages.push(JSON.parse(s))};</script><script>');
  await call('Page.setDocumentContent', { frameId: frameTree.frame.id, html });
  assert.equal(await evaluate('window.messages[0].action'), 'ready');
  const model = { tabs: [{ id: 1, title: 'Nowa karta', url: 'fastium://newtab', home: true, back: false, forward: false, loading: false, sleeping: false, awake: false }],
    active: 1, blocking: true, blocked: 0, downloadsOpen: false, downloads: [], notice: '' };
  await evaluate(`window.render(${JSON.stringify(model)})`);
  assert.equal(await evaluate('document.querySelector("header").getBoundingClientRect().height'), 100);
  assert.equal(await evaluate('document.querySelector("footer").getBoundingClientRect().height'), 28);
  const shot = await call('Page.captureScreenshot', { format: 'png' });
  writeFileSync(resolve(out, 'dashboard.png'), Buffer.from(shot.data, 'base64'));
  await evaluate('document.getElementById("search").value="żółw & Rust #1";document.getElementById("search-form").requestSubmit()');
  assert.equal(await evaluate('window.messages.at(-1).text'), 'https://www.google.com/search?q=' + encodeURIComponent('żółw & Rust #1'));
  const malicious = '<img src=x onerror="window.compromised=true">';
  model.tabs.push({ ...model.tabs[0], id: 2, title: malicious, url: 'https://example.org/', home: false });
  model.active = 2;
  model.notice = malicious;
  model.downloadsOpen = true;
  model.downloads = [{ id: 7, path: 'C:\\Downloads\\' + malicious, received: 128, total: 0, state: 'running', resumable: false, reason: 0 }];
  await evaluate(`window.render(${JSON.stringify(model)})`);
  assert.equal(await evaluate('document.querySelectorAll("img").length'), 0);
  assert.equal(await evaluate('document.getElementById("notice").textContent'), malicious);
  assert.equal(await evaluate('document.querySelector("progress").hasAttribute("value")'), false);
  assert.equal(await evaluate('document.getElementById("downloads").getBoundingClientRect().height'), 220);
  await evaluate('window.focusAddress();document.getElementById("address").value="unfinished query"');
  model.blocked = 3;
  await evaluate(`window.render(${JSON.stringify(model)})`);
  assert.equal(await evaluate('document.getElementById("address").value'), 'unfinished query');
  await evaluate('document.querySelector("[data-action=pause]").click()');
  assert.deepEqual(await evaluate('window.messages.at(-1)'), { action: 'pause', id: 7 });
  await evaluate('document.querySelector(".tab-select").click()');
  assert.deepEqual(await evaluate('window.messages.at(-1)'), { action: 'switch', id: 1 });
  console.log('PASS: dashboard, IPC, search encoding, XSS escaping, download progress/actions, editable omnibox, tab selection.');
  console.log(`Screenshot: ${resolve(out, 'dashboard.png')}`);
} finally {
  for (const item of pending.values()) clearTimeout(item.timer);
  processBrowser.kill();
}
