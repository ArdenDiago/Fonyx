// Endpoint tests: every Tauri command the frontend can call, driven through
// the same bridge the UI tests use (so the command -> D-Bus mapping is
// exercised too), plus the few D-Bus methods that have no Tauri command.
// Also checks the contract between the layers: frontend calls, registered
// Tauri commands and D-Bus methods must all line up.
//
// Only runs under run-isolated.sh. These tests route, mute and re-volume
// streams and outputs — fine against the throwaway session that script
// builds (one fake output, two fake apps), not against someone's real one.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import dbus from 'dbus-next';
import { expect, test } from '@playwright/test';

test.skip(!process.env.FONYX_ISOLATED, 'endpoint tests only run in the isolated session: sh ui/tests/run-isolated.sh');

const here = path.dirname(fileURLToPath(import.meta.url));
const read = rel => fs.readFileSync(path.resolve(here, '..', rel), 'utf8');
const fnNames = src => [...src.matchAll(/pub fn (\w+)\(/g)].map(m => m[1]);
const DAEMON_COMMANDS = fnNames(read('src-tauri/src/commands.rs'));
const SETTINGS_COMMANDS = fnNames(read('src-tauri/src/settings.rs'));
const toPascal = s => s.split('_').map(w => w[0].toUpperCase() + w.slice(1)).join('');

const BASE = 'http://localhost:4571';
const NONE = 4294967295; // u32::MAX, the daemon's "no such id"
const used = new Set();

// One Tauri command, as the frontend would invoke it. Throws on a daemon error.
async function call(cmd, args = {}) {
  used.add(cmd);
  const data = await (await fetch(`${BASE}/__invoke`, { method: 'POST', body: JSON.stringify({ cmd, args }) })).json();
  if (data && data.__error) throw new Error(`${cmd}: ${data.__error}`);
  return data;
}

let bus, iface;
test.beforeAll(async () => {
  bus = dbus.sessionBus();
  iface = (await bus.getProxyObject('org.fonyx.Daemon', '/org/fonyx/Daemon')).getInterface('org.fonyx.Daemon');
});
test.afterAll(() => bus?.disconnect());

const groups = () => call('list_groups');
const groupById = async id => (await groups()).find(g => g[0] === id);
const newGroup = name => call('create_group', { name: `${name}-${Date.now()}` });
const testOutput = async () => (await call('list_outputs')).find(o => o[1] === 'test-output' || o[2] === 'Test Output');
const testStream = async name => (await call('list_streams')).find(s => s[1] === name);
const defaultGroup = async () => (await groups()).find(g => g[4]);

// ---------------------------------------------------------------- contract

test('contract: every Tauri command has a D-Bus method, and the rest are known', () => {
  const dbusMethods = iface.$methods.map(m => m.name);
  const fromCommands = DAEMON_COMMANDS.map(toPascal);
  expect(fromCommands.filter(m => !dbusMethods.includes(m)), 'Tauri commands with no D-Bus method').toEqual([]);
  // Backend-only methods (used by Quickshell or the CLI, or kept without a UI). A new
  // D-Bus method must get a Tauri command or be added here on purpose.
  expect(dbusMethods.filter(m => !fromCommands.includes(m)).sort()).toEqual(['CreateAutoRouteRule', 'CreateDeviceConnectedRule', 'RenameGroup', 'SetGroupFeeds']);
});

test('contract: every command is registered with Tauri, and the frontend only calls real ones', () => {
  const registered = [...read('src-tauri/src/lib.rs').matchAll(/(?:commands|settings)::(\w+)/g)].map(m => m[1]);
  const all = [...DAEMON_COMMANDS, ...SETTINGS_COMMANDS];
  expect(all.filter(c => !registered.includes(c)), 'commands missing from generate_handler!').toEqual([]);
  const invoked = [...new Set([...read('index.html').matchAll(/invoke\('(\w+)'/g)].map(m => m[1]))];
  expect(invoked.filter(c => !all.includes(c)), 'frontend invokes a command that does not exist').toEqual([]);
});

// ------------------------------------------------------------------ groups

test('groups: create, list, gain, delete; Default and Cue cannot be deleted', async () => {
  const name = `ApiGroup-${Date.now()}`;
  const id = await call('create_group', { name });
  expect(await groupById(id)).toEqual([id, name, 1, [], false]);

  await call('set_gain', { groupId: id, value: 0.5 });
  expect((await groupById(id))[2]).toBeCloseTo(0.5);

  await call('delete_group', { groupId: id });
  expect(await groupById(id)).toBeUndefined();

  const def = await defaultGroup();
  const cue = await call('get_cue_group_id');
  await call('delete_group', { groupId: def[0] });
  await call('delete_group', { groupId: cue });
  expect(await groupById(def[0])).toBeDefined();
  expect(await groupById(cue)).toBeDefined();
});

test('master gain: set and read back', async () => {
  const before = await call('get_master_gain');
  await call('set_master_gain', { value: 0.7 });
  expect(await call('get_master_gain')).toBeCloseTo(0.7);
  await call('set_master_gain', { value: before });
});

test('sub-groups (D-Bus only): feed, refuse a second level, release', async () => {
  const [parent, child] = [await newGroup('ApiParent'), await newGroup('ApiChild')];
  await iface.SetGroupFeeds(child, parent);
  expect(await call('list_group_feeds')).toContainEqual([child, parent]);
  await expect(iface.SetGroupFeeds(parent, child)).rejects.toThrow(/sub-group/);
  await iface.SetGroupFeeds(child, NONE);
  expect(await call('list_group_feeds')).not.toContainEqual([child, parent]);

  // Deleting a parent releases its sub-groups.
  await iface.SetGroupFeeds(child, parent);
  await call('delete_group', { groupId: parent });
  expect((await call('list_group_feeds')).some(([c]) => c === child)).toBe(false);
  await call('delete_group', { groupId: child });
});

// --------------------------------------------------------------------- DSP

test('dsp: EQ band, filters, compressor and gate round-trip through get_group_details', async () => {
  const id = await newGroup('ApiDsp');
  await call('set_eq_band', { groupId: id, bandIndex: 0, freq: 250, gain: 3, q: 1.5, enabled: true });
  await call('set_highpass', { groupId: id, freq: 120, q: 0.7, enabled: true });
  await call('set_lowpass', { groupId: id, freq: 9000, q: 0.9, enabled: true });
  await call('set_compressor', { groupId: id, thresholdDb: -24, ratio: 4, attackMs: 5, releaseMs: 80, makeupGainDb: 2, enabled: true });
  await call('set_gate', { groupId: id, thresholdDb: -45, attackMs: 3, releaseMs: 120, holdMs: 40, enabled: true });

  const [bands, hp, lp, comp, gate] = await call('get_group_details', { groupId: id });
  const close = (got, want) => want.forEach((w, i) => (typeof w === 'number' ? expect(got[i]).toBeCloseTo(w, 3) : expect(got[i]).toBe(w)));
  expect(bands).toHaveLength(6);
  close(bands[0], [250, 3, 1.5, true]);
  close(hp, [120, 0.7, true]);
  close(lp, [9000, 0.9, true]);
  close(comp, [-24, 4, 5, 80, 2, true]);
  close(gate, [-45, 3, 120, 40, true]);
  await call('delete_group', { groupId: id });
});

test('dsp: EQ level and pan are clamped and read back', async () => {
  const id = await newGroup('ApiPan');
  await call('set_eq_level', { groupId: id, db: 6 });
  expect(await call('get_eq_level', { groupId: id })).toBeCloseTo(6);
  await call('set_eq_level', { groupId: id, db: 99 });
  expect(await call('get_eq_level', { groupId: id })).toBeCloseTo(18);

  await call('set_pan', { groupId: id, pan: -0.25, mono: false });
  expect((await call('list_group_pans')).find(p => p[0] === id)).toEqual([id, -0.25, false]);
  await call('set_pan', { groupId: id, pan: 5, mono: true });
  expect((await call('list_group_pans')).find(p => p[0] === id)).toEqual([id, 1, true]);
  await call('delete_group', { groupId: id });
});

test('plugins: list types, add, set a parameter, bypass, reorder, remove', async () => {
  const available = await call('list_available_plugins');
  expect(available.map(p => p[0]).sort()).toEqual(['deep_bass', 'mids', 'soft_clipper', 'stereo_widener']);

  const id = await newGroup('ApiPlugins');
  await call('add_group_plugin', { groupId: id, pluginType: 'soft_clipper' });
  await call('add_group_plugin', { groupId: id, pluginType: 'mids' });
  let chain = await call('list_group_plugins', { groupId: id });
  expect(chain.map(s => [s[0], s[1], s[2]])).toEqual([[0, 'soft_clipper', true], [1, 'mids', true]]);

  const [paramName, min, max] = available.find(p => p[0] === 'soft_clipper')[2][0];
  const value = (min + max) / 2;
  await call('set_group_plugin_param', { groupId: id, index: 0, paramName, value });
  await call('set_group_plugin_bypass', { groupId: id, index: 0, bypassed: true });
  chain = await call('list_group_plugins', { groupId: id });
  expect(chain[0][2]).toBe(false);
  expect(chain[0][4][chain[0][3].indexOf(paramName)]).toBeCloseTo(value, 3);

  await call('reorder_group_plugin', { groupId: id, fromIndex: 0, toIndex: 1 });
  expect((await call('list_group_plugins', { groupId: id })).map(s => s[1])).toEqual(['mids', 'soft_clipper']);
  await call('remove_group_plugin', { groupId: id, index: 0 });
  expect((await call('list_group_plugins', { groupId: id })).map(s => s[1])).toEqual(['soft_clipper']);
  await call('delete_group', { groupId: id });
});

// --------------------------------------------------------------- live data

test('live data: levels, spectrum, metrics, health and graph have the documented shapes', async () => {
  const id = await newGroup('ApiLive');
  await expect.poll(async () => (await call('get_levels')).some(l => l[0] === id)).toBe(true);
  for (const row of await call('get_levels')) {
    expect(row).toHaveLength(5);
    row.forEach(v => expect(typeof v).toBe('number'));
  }
  expect(await call('get_spectrum', { groupId: id })).toHaveLength(24);
  expect(await call('get_spectrum', { groupId: 999999 })).toEqual([]);

  const metrics = await call('get_metrics');
  expect(metrics).toHaveLength(4);
  metrics.forEach(v => expect(typeof v).toBe('number'));

  const [score, checks] = await call('run_health_checks');
  expect(['ok', 'warning', 'error']).toContain(score);
  expect(checks.map(c => c[0])).toEqual(expect.arrayContaining(['PipeWire connection', 'Groups', 'Outputs', 'D-Bus']));

  const [nodes, edges] = await call('get_graph');
  expect(nodes.some(n => n[1].startsWith('ApiLive-'))).toBe(true);
  expect(Array.isArray(edges)).toBe(true);
  await call('delete_group', { groupId: id });
});

// ----------------------------------------------------------------- streams

test('streams: discovered, routed, isolated left/right, muted, returned to Default', async () => {
  await expect.poll(async () => (await call('list_streams')).map(s => s[1]).filter(n => n.startsWith('TestApp')).sort()).toEqual(['TestAppAlpha', 'TestAppBeta']);
  const [sid, , , , appId] = await testStream('TestAppAlpha');
  expect(appId).toBe('appname:TestAppAlpha');
  const def = await defaultGroup();
  await expect.poll(async () => (await defaultGroup())[3]).toContain(sid); // auto-routed into Default

  const id = await newGroup('ApiRoute');
  await call('route_stream_to_group', { streamId: sid, groupId: id });
  await expect.poll(async () => (await groupById(id))[3]).toEqual([sid]);
  expect((await defaultGroup())[3]).not.toContain(sid);

  await call('set_stream_side', { streamId: sid, side: 'L' });
  expect(await call('list_stream_sides')).toContainEqual([sid, 'L']);
  await call('set_stream_side', { streamId: sid, side: 'R' });
  expect(await call('list_stream_sides')).toContainEqual([sid, 'R']);
  await call('set_stream_side', { streamId: sid, side: 'S' });
  expect((await call('list_stream_sides')).some(s => s[0] === sid)).toBe(false);

  expect(await call('set_stream_muted', { streamId: sid, muted: true })).toBe(true);
  expect(await call('set_stream_muted', { streamId: sid, muted: false })).toBe(true);

  const levels = await call('get_stream_levels');
  expect(levels.find(l => l[0] === sid)[1]).toBeGreaterThanOrEqual(0);

  await call('unroute_stream', { streamId: sid });
  await expect.poll(async () => (await defaultGroup())[3]).toContain(sid);
  expect((await groupById(id))[3]).toEqual([]);
  await call('delete_group', { groupId: id });
  expect(def[0]).toBe((await defaultGroup())[0]);
});

test('streams: deleting a group sends its streams back to Default', async () => {
  const [sid] = await testStream('TestAppBeta');
  const id = await newGroup('ApiOrphan');
  await call('route_stream_to_group', { streamId: sid, groupId: id });
  await expect.poll(async () => (await groupById(id))[3]).toEqual([sid]);
  await call('delete_group', { groupId: id });
  await expect.poll(async () => (await defaultGroup())[3]).toContain(sid);
});

test('streams: profile save and Own DSP', async () => {
  const [sid] = await testStream('TestAppAlpha');
  expect(await call('save_profile_for_stream', { streamId: sid, label: 'Alpha profile' })).toBe(true);
  expect(await call('save_profile_for_stream', { streamId: 999999, label: 'nobody' })).toBe(false);

  const strip = await call('create_app_strip', { streamId: sid });
  const group = await groupById(strip);
  expect(group[1]).toBe('TestAppAlpha');
  await expect.poll(async () => (await groupById(strip))[3]).toContain(sid);
  await expect(call('create_app_strip', { streamId: 999999 })).rejects.toThrow(/no stream/);
  await call('delete_group', { groupId: strip });
  await expect.poll(async () => (await defaultGroup())[3]).toContain(sid);
});

// ----------------------------------------------------------------- outputs

test('outputs: discovery, device volume and mute', async () => {
  await expect.poll(async () => !!(await testOutput())).toBe(true);
  const out = await testOutput();
  expect(out[4], 'available').toBe(true);
  const oid = out[0];

  expect(await call('set_output_volume', { outputId: oid, volume: 0.5 })).toBe(true);
  await expect.poll(async () => (await call('get_output_volume', { outputId: oid }))[0]).toBeCloseTo(0.5, 2);
  expect(await call('set_output_muted', { outputId: oid, muted: true })).toBe(true);
  await expect.poll(async () => (await call('get_output_volume', { outputId: oid }))[1]).toBe(true);
  expect(await call('set_output_muted', { outputId: oid, muted: false })).toBe(true);
  expect(await call('set_output_volume', { outputId: oid, volume: 1 })).toBe(true);
  expect(await call('set_output_volume', { outputId: 'no-such-output', volume: 1 })).toBe(false);
});

test('outputs: group routes with gain and sync', async () => {
  const oid = (await testOutput())[0];
  const id = await newGroup('ApiRoutes');
  expect(await call('list_group_output_routes', { groupId: id })).toEqual([]);

  await call('set_group_output_route', { groupId: id, outputId: oid, gain: 0.5, enabled: true });
  let [route] = await call('list_group_output_routes', { groupId: id });
  expect([route[0], route[1], route[2], route[3], route[4]]).toEqual([oid, 0.5, true, 'explicit', 'off']);

  await call('set_output_synchronization', { groupId: id, outputId: oid, syncMode: 'manual', manualDelayMs: 20 });
  [route] = await call('list_group_output_routes', { groupId: id });
  expect([route[4], route[5]]).toEqual(['manual', 20]);

  await call('remove_group_output_route', { groupId: id, outputId: oid });
  expect(await call('list_group_output_routes', { groupId: id })).toEqual([]);
  await call('delete_group', { groupId: id });
});

test('outputs: Master routes with gain and sync', async () => {
  const oid = (await testOutput())[0];
  await call('set_master_output_route', { outputId: oid, gain: 0.25, enabled: true });
  let route = (await call('list_master_output_routes')).find(r => r[0] === oid);
  expect([route[1], route[2]]).toEqual([0.25, true]);
  await call('set_master_output_synchronization', { outputId: oid, syncMode: 'automatic', manualDelayMs: 0 });
  route = (await call('list_master_output_routes')).find(r => r[0] === oid);
  expect(route[4]).toBe('automatic');
  await call('remove_master_output_route', { outputId: oid });
  expect((await call('list_master_output_routes')).some(r => r[0] === oid)).toBe(false);
});

test('outputs: preferred device, buffer size and max compensation are stored and clamped', async () => {
  const oid = (await testOutput())[0];
  await call('set_preferred_default_output', { outputId: oid });
  expect(await call('get_preferred_default_output')).toBe(oid);
  await call('set_preferred_default_output', { outputId: '' });
  expect(await call('get_preferred_default_output')).toBe('');

  const buffer = await call('get_buffer_ms');
  await call('set_buffer_ms', { value: 1 });
  expect(await call('get_buffer_ms')).toBe(10);
  await call('set_buffer_ms', { value: 9999 });
  expect(await call('get_buffer_ms')).toBe(150);
  await call('set_buffer_ms', { value: buffer });

  const max = await call('get_max_compensation_ms');
  await call('set_max_compensation_ms', { value: 9999 });
  expect(await call('get_max_compensation_ms')).toBe(500);
  await call('set_max_compensation_ms', { value: max });
});

test('cue: cue group, cue outputs and duck level', async () => {
  const cue = await call('get_cue_group_id');
  expect(await groupById(cue)).toBeDefined();
  const oid = (await testOutput())[0];
  await call('set_cue_output', { outputId: oid, enabled: true });
  expect(await call('list_cue_outputs')).toContain(oid);
  await call('set_cue_output', { outputId: oid, enabled: false });
  expect(await call('list_cue_outputs')).not.toContain(oid);

  const level = await call('get_cue_duck_level');
  await call('set_cue_duck_level', { value: 0.5 });
  expect(await call('get_cue_duck_level')).toBeCloseTo(0.5);
  await call('set_cue_duck_level', { value: level });
});

// ------------------------------------------------------------------ scenes

test('scenes: capture, update, duplicate, activate, delete', async () => {
  const id = await newGroup('ApiScene');
  await call('set_gain', { groupId: id, value: 0.3 });
  const scene = await call('capture_current_as_scene', { name: 'ApiSceneA' });
  expect((await call('list_scenes')).find(s => s[0] === scene)[1]).toBe('ApiSceneA');

  await call('set_gain', { groupId: id, value: 0.9 });
  await call('activate_scene', { sceneId: scene });
  await expect.poll(async () => (await groupById(id))[2]).toBeCloseTo(0.3);

  expect(await call('update_scene', { sceneId: scene })).toBe(true);
  expect(await call('update_scene', { sceneId: 999999 })).toBe(false);
  const copy = await call('duplicate_scene', { sceneId: scene, newName: 'ApiSceneB' });
  expect((await call('list_scenes')).find(s => s[0] === copy)[1]).toBe('ApiSceneB');
  expect(await call('duplicate_scene', { sceneId: 999999, newName: 'x' })).toBe(NONE);

  await call('delete_scene', { sceneId: scene });
  await call('delete_scene', { sceneId: copy });
  expect((await call('list_scenes')).some(s => s[0] === scene || s[0] === copy)).toBe(false);
  await call('delete_group', { groupId: id });
});

// ------------------------------------------------------------------- rules

test('rules: save, read, list, disable, history, validation, delete', async () => {
  const id = await newGroup('ApiRule');
  const rule = { id: NONE, name: 'ApiRule', enabled: true, priority: 3, cooldown_ms: 250, trigger: { AppStarted: { app_id: 'appname:Nobody', service: null } }, conditions: [], actions: [{ RouteTriggeringAppToGroup: { group_id: id } }] };
  const ruleId = await call('save_rule', { json: JSON.stringify(rule) });
  expect(JSON.parse(await call('get_rule', { ruleId })).name).toBe('ApiRule');
  expect(await call('get_rule', { ruleId: 999999 })).toBe('');
  expect((await call('list_rules')).find(r => r[0] === ruleId).slice(0, 5)).toEqual([ruleId, 'ApiRule', true, 3, 250]);

  await call('set_rule_enabled', { ruleId, enabled: false });
  expect((await call('list_rules')).find(r => r[0] === ruleId)[2]).toBe(false);
  expect(Array.isArray(await call('rule_history'))).toBe(true);

  // An app action needs an app trigger; a rule may not name a missing group.
  await expect(call('save_rule', { json: JSON.stringify({ ...rule, trigger: 'SystemResume' }) })).rejects.toThrow();
  await expect(call('save_rule', { json: JSON.stringify({ ...rule, actions: [{ SetGroupVolume: { group_id: 999999, gain: 1 } }] }) })).rejects.toThrow(/no group/);
  await expect(call('save_rule', { json: 'not json' })).rejects.toThrow();

  await call('delete_rule', { ruleId });
  expect((await call('list_rules')).some(r => r[0] === ruleId)).toBe(false);
  await call('delete_group', { groupId: id });
});

test('rules (D-Bus only): the two template rules can be created', async () => {
  const id = await newGroup('ApiAuto');
  const ruleId = await iface.CreateAutoRouteRule('ApiAutoRule', 'route_app_to_group', 'appname:TestAppBeta', '', id);
  expect((await call('list_rules')).find(r => r[0] === ruleId)[1]).toBe('ApiAutoRule');
  const deviceRule = await iface.CreateDeviceConnectedRule('ApiDeviceRule', (await testOutput())[0], id, 0.5);
  expect((await call('list_rules')).find(r => r[0] === deviceRule)[1]).toBe('ApiDeviceRule');
  await call('delete_rule', { ruleId });
  await call('delete_rule', { ruleId: deviceRule });
  await call('delete_group', { groupId: id });
});

test('ducking: create, list, disable, delete', async () => {
  const [a, b] = [await newGroup('ApiDuckA'), await newGroup('ApiDuckB')];
  const ruleId = await call('create_ducking_rule', { triggerGroup: a, targetGroups: [b], reductionDb: 12, attackMs: 50, holdMs: 200, releaseMs: 500, thresholdDb: -40 });
  expect((await call('list_ducking_rules')).find(r => r[0] === ruleId)).toEqual([ruleId, true, a, [b], 12, 50, 200, 500, -40]);
  await call('set_ducking_rule_enabled', { ruleId, enabled: false });
  expect((await call('list_ducking_rules')).find(r => r[0] === ruleId)[1]).toBe(false);
  await call('delete_ducking_rule', { ruleId });
  expect((await call('list_ducking_rules')).some(r => r[0] === ruleId)).toBe(false);
  await call('delete_group', { groupId: a });
  await call('delete_group', { groupId: b });
});

// --------------------------------------------------------------- recording

test('recording: each format writes a file; a bad format is refused', async () => {
  const id = await newGroup('ApiRec');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mtx-rec-'));
  for (const [format, ext] of [['wav16', 'wav'], ['wav32', 'wav'], ['flac', 'flac']]) {
    const file = path.join(dir, `${format}.${ext}`);
    const rec = await call('start_recording', { groupId: id, path: file, format });
    expect(rec).not.toBe(NONE);
    await expect.poll(async () => (await call('list_recordings')).find(r => r[0] === rec)?.slice(0, 3)).toEqual([rec, id, file]);
    await call('stop_recording', { id: rec });
    await expect.poll(() => fs.existsSync(file) && fs.statSync(file).size > 0).toBe(true);
    await expect.poll(async () => (await call('list_recordings')).some(r => r[0] === rec)).toBe(false);
  }
  expect(await call('start_recording', { groupId: id, path: path.join(dir, 'x.ogg'), format: 'opus' })).toBe(NONE);
  expect(await call('start_recording', { groupId: 999999, path: path.join(dir, 'y.wav'), format: 'wav16' })).toBe(NONE);
  fs.rmSync(dir, { recursive: true });
  await call('delete_group', { groupId: id });
});

// ---------------------------------------------------------------- settings

test('settings: autostart state is readable', async () => {
  expect(typeof (await call('get_autostart'))).toBe('boolean');
  expect(typeof (await call('get_quickshell_autostart'))).toBe('boolean');
});

// ---------------------------------------------------------------- coverage

test('coverage: every command was exercised by this file', () => {
  // The two setters flip real systemd units; there is no systemd user
  // session here. smoke.spec.mjs covers the toggle against a real session.
  const exempt = ['set_autostart', 'set_quickshell_autostart'];
  const missing = [...DAEMON_COMMANDS, ...SETTINGS_COMMANDS].filter(c => !used.has(c) && !exempt.includes(c));
  expect(missing, 'commands with no endpoint test').toEqual([]);
});
