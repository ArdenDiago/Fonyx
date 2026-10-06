// Test-only bridge: serves the real ui/index.html over HTTP (so it runs in
// a plain browser under Playwright, same-origin, no CORS games) and proxies
// its `window.__TAURI__.core.invoke()` calls straight to the real, live
// `org.sonicforge.Daemon` D-Bus service — the same service the real Tauri
// backend talks to. This is not a mock: it exercises the actual daemon.
//
// The Tauri <-> D-Bus mapping is generated from `commands.rs` itself
// (regex-parsed) rather than hand-duplicated, so it can't silently drift
// out of sync with the real command list — a command added there is
// covered here automatically as long as its D-Bus method name is the
// PascalCase of its Rust name (true for every command in this file today).
import { execFile } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import dbus from 'dbus-next';

// Mirrors ui/src-tauri/src/settings.rs exactly — these two commands don't
// go through the daemon at all, they shell out to systemctl directly, so
// there's no D-Bus method to proxy to.
const AUTOSTART_UNITS = { get_autostart: 'sonicforge-daemon.service', get_quickshell_autostart: 'sonicforge-quickshell.service' };
function systemctl(args) {
  return new Promise(resolve => {
    execFile('systemctl', args, err => resolve(!err));
  });
}

const here = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.resolve(here, '..');
const indexHtml = fs.readFileSync(path.join(uiDir, 'index.html'), 'utf8');
const commandsSrc = fs.readFileSync(path.join(uiDir, 'src-tauri/src/commands.rs'), 'utf8');

// `#[tauri::command]` or `#[tauri::command(async)]`, then `\n[#[allow(...)]\n]*pub fn name(state: State<...>, arg1: Type, ...)`
// — good enough for this file's consistent formatting, not a general Rust parser.
const commandArgs = {};
const fnRe = /#\[tauri::command(?:\(async\))?\]\s*(?:#\[[^\]]*\]\s*)*pub fn (\w+)\(([^)]*)\)/g;
let m;
while ((m = fnRe.exec(commandsSrc))) {
  const [, name, paramsRaw] = m;
  const argNames = paramsRaw
    .split(',')
    .map(p => p.trim())
    .filter(p => p && !p.startsWith('state'))
    .map(p => p.split(':')[0].trim());
  commandArgs[name] = argNames;
}

const toPascalCase = snake => snake.split('_').map(w => w[0].toUpperCase() + w.slice(1)).join('');
const toCamelCase = snake => {
  const [first, ...rest] = snake.split('_');
  return first + rest.map(w => w[0].toUpperCase() + w.slice(1)).join('');
};

const SHIM = `
  window.__TAURI__ = { core: { invoke: async (cmd, args) => {
    const res = await fetch('/__invoke', { method: 'POST', body: JSON.stringify({ cmd, args }) });
    const data = await res.json();
    if (data && data.__error) throw new Error(data.__error);
    return data;
  } } };
`;

async function main() {
  const bus = dbus.sessionBus();
  const obj = await bus.getProxyObject('org.sonicforge.Daemon', '/org/sonicforge/Daemon');
  const iface = obj.getInterface('org.sonicforge.Daemon');

  const server = http.createServer((req, res) => {
    if (req.method === 'GET' && (req.url === '/' || req.url === '/index.html')) {
      res.writeHead(200, { 'content-type': 'text/html' });
      // Shim must run before index.html's own <script> — inject right
      // after <head> opens.
      res.end(indexHtml.replace('<head>', `<head><script>${SHIM}</script>`));
      return;
    }
    if (req.method === 'POST' && req.url === '/__invoke') {
      let body = '';
      req.on('data', c => (body += c));
      req.on('end', async () => {
        try {
          const { cmd, args } = JSON.parse(body || '{}');
          // Not daemon state — same systemctl calls settings.rs itself
          // makes, no D-Bus involved.
          if (cmd in AUTOSTART_UNITS) {
            return respond(res, await systemctl(['--user', 'is-enabled', '--quiet', AUTOSTART_UNITS[cmd]]));
          }
          if (cmd === 'set_autostart' || cmd === 'set_quickshell_autostart') {
            const unit = cmd === 'set_autostart' ? AUTOSTART_UNITS.get_autostart : AUTOSTART_UNITS.get_quickshell_autostart;
            const ok = await systemctl(['--user', (args || {}).enabled ? 'enable' : 'disable', '--quiet', unit]);
            if (!ok) throw new Error(`systemctl --user ${(args || {}).enabled ? 'enable' : 'disable'} ${unit} failed`);
            return respond(res, null);
          }
          const argNames = commandArgs[cmd];
          if (!argNames) throw new Error(`bridge: unmapped command "${cmd}"`);
          const positional = argNames.map(n => (args || {})[toCamelCase(n)]);
          const member = toPascalCase(cmd);
          const fn = iface[member];
          if (typeof fn !== 'function') throw new Error(`bridge: no D-Bus method "${member}" for command "${cmd}"`);
          const result = await fn.apply(iface, positional);
          respond(res, result === undefined ? null : result);
        } catch (err) {
          respond(res, { __error: String((err && err.message) || err) }, 500);
        }
      });
      return;
    }
    res.writeHead(404);
    res.end();
  });

  // Stringify *before* writing any header — dbus-next represents a D-Bus
  // u64 (e.g. a rule's cooldown_ms) as a JS BigInt, which JSON.stringify
  // throws on; if headers were already sent by the time that throws, the
  // fallback error response crashes the whole server trying to send a
  // second set of headers. Coercing BigInt -> Number here is safe for
  // every value this app actually produces (ids, counts, ms durations —
  // nowhere near Number.MAX_SAFE_INTEGER).
  function respond(res, value, status = 200) {
    const body = JSON.stringify(value, (_key, v) => (typeof v === 'bigint' ? Number(v) : v));
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(body);
  }

  const port = process.env.BRIDGE_PORT ? Number(process.env.BRIDGE_PORT) : 4571;
  server.listen(port, () => {
    console.log(`bridge listening on http://localhost:${port} (${Object.keys(commandArgs).length} commands mapped)`);
  });
}

main().catch(err => {
  console.error('bridge failed to start:', err);
  process.exit(1);
});
