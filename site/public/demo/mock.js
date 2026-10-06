// A pretend SonicForge daemon for the website's "try it" demo.
//
// The real desktop UI (ui/index.html) reaches the mixer through one function,
// window.__TAURI__.core.invoke(command, args). This file supplies that function
// and answers every command from memory, in the same positional-tuple shapes
// the real daemon returns. Nothing is processed and nothing makes a sound:
// the meters follow made-up signals. Reload and it all starts over.
(() => {
  // The UI keeps mutes, links and the theme in localStorage. Hand it a
  // throwaway one, so the site stores nothing in the visitor's browser.
  const mem = new Map();
  Object.defineProperty(window, 'localStorage', {
    value: {
      getItem: (k) => (mem.has(k) ? mem.get(k) : null),
      setItem: (k, v) => void mem.set(k, String(v)),
      removeItem: (k) => void mem.delete(k),
      clear: () => mem.clear(),
    },
  });
  // Two fader groups to start with, so the Faders tab shows its coloured strips.
  mem.set('mtx-fader-groups', JSON.stringify([
    { id: 1, name: 'Band', gainDb: 0, muted: false, members: [1, 4] },
    { id: 2, name: 'Voices', gainDb: 0, muted: false, members: [3, 5] },
  ]));

  // demo.css draws the vertical faders by hand; tell each one how full its track is.
  // The app re-renders its strips whenever it likes, so this simply keeps up.
  setInterval(() => {
    for (const el of document.querySelectorAll('.fader-col input[type="range"], .master-col input[type="range"]')) {
      el.style.setProperty('--fill', `${((el.value - el.min) / (el.max - el.min)) * 100}%`);
    }
  }, 60);

  const NONE = 4294967295; // u32::MAX, the daemon's "no such id"
  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
  const db = (lin) => (lin > 1e-5 ? 20 * Math.log10(lin) : -100);
  const lin = (d) => Math.pow(10, d / 20);
  const copy = (v) => JSON.parse(JSON.stringify(v));
  const t0 = performance.now();
  const now = () => (performance.now() - t0) / 1000;

  let nextId = 10;
  const newGroup = (id, name, isProtected = false) => ({
    id, name, gain: 1, protected: isProtected, feeds: NONE,
    bands: [100, 300, 1000, 3000, 8000, 12000].map((f) => [f, 0, 1, true]),
    hp: [80, 0.707, false], lp: [80, 0.707, false],
    comp: [-18, 2, 10, 100, 0, false], gate: [-50, 2, 150, 50, false],
    eqLevel: 0, pan: 0, mono: false, plugins: [], routes: [],
  });
  const route = (outputId, gain = 1) => ({ outputId, gain, enabled: true, sync: 'off', delay: 0 });

  const S = {
    master: 1, masterRoutes: [], groups: [newGroup(1, 'Default', true), newGroup(2, 'Cue', true), newGroup(3, 'Browser'), newGroup(4, 'Music'), newGroup(5, 'Chat')],
    // shape: where each pretend app's energy sits (low, mid, high), for the spectrum
    streams: [
      { id: 61, name: 'Web Browser', service: 'Conference talk', app: 'appname:Web Browser', group: 3, playing: true, shape: [0.3, 1, 0.5] },
      { id: 62, name: 'Music Player', service: 'Lo-fi radio', app: 'appname:Music Player', group: 4, playing: true, shape: [1, 0.7, 0.6] },
      { id: 63, name: 'Voice Chat', service: 'Team call', app: 'appname:Voice Chat', group: 5, playing: true, shape: [0.2, 1, 0.3] },
      { id: 64, name: 'Game', service: 'Level 3', app: 'appname:Game', group: 1, playing: true, shape: [0.9, 0.6, 0.9] },
      { id: 65, name: 'Video Player', service: '', app: 'appname:Video Player', group: 1, playing: false, shape: [0.6, 0.8, 0.5] },
    ],
    outputs: [
      { id: 'demo:speakers', name: 'Speakers', desc: 'Built-in Audio Analog Stereo', kind: 'speaker', available: true, isDefault: true, volume: 0.8, muted: false },
      { id: 'demo:headphones', name: 'Headphones', desc: 'USB Headphone Amp', kind: 'headphones', available: true, isDefault: false, volume: 0.6, muted: false },
      { id: 'demo:bluetooth', name: 'Living room', desc: 'Bluetooth Speaker', kind: 'bluetooth', available: true, isDefault: false, volume: 0.7, muted: false },
      { id: 'demo:hdmi', name: 'Desk monitor', desc: 'HDMI Audio (unplugged)', kind: 'hdmi', available: false, isDefault: false, volume: 1, muted: false },
    ],
    cueOutputs: [], cueDuck: 0.2, preferred: '', bufferMs: 85, maxCompMs: 150,
    scenes: [], rules: [], ducking: [], recordings: [], autostart: true, quickshell: false,
  };
  S.groups[2].gain = 0.55; S.groups[3].gain = 0.8; S.groups[4].gain = 0.7;
  S.groups[3].routes.push(route('demo:speakers'), route('demo:bluetooth', 0.6));

  const PLUGINS = [
    ['soft_clipper', 'Soft Clipper', [['drive', 1, 10, 1]]],
    ['stereo_widener', 'Stereo Widener', [['width', 0, 2, 1]]],
    ['deep_bass', 'Deep Bass', [['amount', 0, 12, 0], ['freq', 40, 200, 80]]],
    ['mids', 'Mids', [['amount', -12, 12, 0], ['freq', 300, 4000, 1000]]],
  ];

  const group = (id) => S.groups.find((g) => g.id === id);
  const stream = (id) => S.streams.find((s) => s.id === id);
  const output = (id) => S.outputs.find((o) => o.id === id);
  const must = (thing, what) => { if (!thing) throw `InvalidArgs: no such ${what}`; return thing; };
  const routesOf = (list) => list.map((r) => [r.outputId, r.gain, r.enabled, 'explicit', r.sync, r.delay, r.sync === 'automatic']);
  const setRoute = (list, outputId, gain, enabled) => {
    const r = list.find((x) => x.outputId === outputId);
    r ? Object.assign(r, { gain, enabled }) : list.push({ ...route(outputId, gain), enabled });
  };
  const dropRoute = (list, outputId) => list.splice(0, list.length, ...list.filter((r) => r.outputId !== outputId));

  // ----- Made-up signals -----
  const wob = (f, o) => 0.5 + 0.5 * Math.sin(now() * f + o);
  function streamLevel(s) {
    if (!s.playing || s.muted) return 0;
    switch (s.id) {
      case 61: return 0.3 + 0.3 * wob(7, 0) * wob(3.1, 1);                                   // a talk: steady speech
      case 62: return 0.45 + 0.35 * Math.pow(1 - ((now() * 2.1) % 1), 3);                    // music: a beat
      case 63: return (wob(0.45, 0) * wob(0.23, 2) > 0.22 ? 1 : 0) * (0.35 + 0.4 * wob(9, 0)); // a call: someone talks, then doesn't
      case 64: return 0.15 + 0.7 * Math.pow(wob(0.9, 3) * wob(1.7, 1), 2);                   // a game: bursts
      default: return 0.4;
    }
  }
  const inputLevel = (g) => Math.min(1, Math.hypot(
    ...S.streams.filter((s) => s.group === g.id).map(streamLevel),
    ...S.groups.filter((c) => c.feeds === g.id).map((c) => outputLevel(c).peak)));

  function duckGain(g) {
    let d = 1;
    for (const r of S.ducking) {
      const trigger = group(r.trigger);
      if (r.enabled && trigger && r.targets.includes(g.id) && db(inputLevel(trigger)) > r.threshold) d = Math.min(d, lin(-r.reduction));
    }
    return d;
  }
  function outputLevel(g) {
    let level = inputLevel(g) * g.gain * S.master * lin(g.eqLevel) * duckGain(g);
    const [gThr, , , , gOn] = g.gate;
    const gateGr = gOn && db(level) < gThr ? -40 : 0;
    const [cThr, ratio, , , makeup, cOn] = g.comp;
    const compGr = cOn && db(level) > cThr ? -(db(level) - cThr) * (1 - 1 / ratio) : 0;
    level = Math.min(0.97, level * lin(compGr + gateGr + makeup)); // the limiter's ceiling
    return { peak: level, compGr, gateGr };
  }

  // ----- Scenes -----
  const snapshot = () => copy({ groups: S.groups, master: S.master, masterRoutes: S.masterRoutes, placement: S.streams.map((s) => [s.id, s.group]) });
  const describeRule = (r) => {
    const name = (x) => (typeof x === 'string' ? x : Object.keys(x)[0]);
    return [r.id, r.name, r.enabled, r.priority, r.cooldown_ms, name(r.trigger), r.actions.map(name).join(', ')];
  };

  // ----- The interface: one function per daemon command -----
  const api = {
    list_groups: () => S.groups.map((g) => [g.id, g.name, g.gain, S.streams.filter((s) => s.group === g.id).map((s) => s.id), g.protected]),
    create_group: ({ name }) => { const g = newGroup(nextId++, name); S.groups.push(g); return g.id; },
    delete_group: ({ groupId }) => {
      if (must(group(groupId), 'group').protected) throw 'InvalidArgs: this group cannot be deleted';
      S.streams.forEach((s) => s.group === groupId && (s.group = 1));
      S.groups.forEach((g) => g.feeds === groupId && (g.feeds = NONE));
      S.groups = S.groups.filter((g) => g.id !== groupId);
    },
    list_group_feeds: () => S.groups.filter((g) => g.feeds !== NONE).map((g) => [g.id, g.feeds]),
    list_streams: () => S.streams.map((s) => [s.id, s.name, s.service, s.playing, s.app]),
    get_stream_levels: () => S.streams.map((s) => [s.id, streamLevel(s)]),
    route_stream_to_group: ({ streamId, groupId }) => { must(group(groupId), 'group'); must(stream(streamId), 'stream').group = groupId; },
    unroute_stream: ({ streamId }) => { must(stream(streamId), 'stream').group = 1; },
    set_stream_muted: ({ streamId, muted }) => { must(stream(streamId), 'stream').muted = muted; return true; },
    set_stream_side: ({ streamId, side }) => { must(stream(streamId), 'stream').side = side === 'L' || side === 'R' ? side : ''; },
    list_stream_sides: () => S.streams.filter((s) => s.side).map((s) => [s.id, s.side]),
    create_app_strip: ({ streamId }) => {
      const s = must(stream(streamId), 'stream');
      const g = { ...copy(group(s.group)), id: nextId++, name: s.name, protected: false };
      S.groups.push(g);
      S.streams.forEach((x) => x.app === s.app && (x.group = g.id));
      return g.id;
    },
    save_profile_for_stream: () => true,

    set_gain: ({ groupId, value }) => { must(group(groupId), 'group').gain = value; },
    set_master_gain: ({ value }) => { S.master = value; },
    get_master_gain: () => S.master,
    set_eq_band: ({ groupId, bandIndex, freq, gain, q, enabled }) => { must(group(groupId), 'group').bands[bandIndex] = [freq, gain, q, enabled]; },
    set_eq_level: ({ groupId, db: d }) => { must(group(groupId), 'group').eqLevel = clamp(d, -18, 18); },
    get_eq_level: ({ groupId }) => group(groupId)?.eqLevel ?? 0,
    set_highpass: ({ groupId, freq, q, enabled }) => { must(group(groupId), 'group').hp = [freq, q, enabled]; },
    set_lowpass: ({ groupId, freq, q, enabled }) => { must(group(groupId), 'group').lp = [freq, q, enabled]; },
    set_compressor: ({ groupId, thresholdDb, ratio, attackMs, releaseMs, makeupGainDb, enabled }) => { must(group(groupId), 'group').comp = [thresholdDb, ratio, attackMs, releaseMs, makeupGainDb, enabled]; },
    set_gate: ({ groupId, thresholdDb, attackMs, releaseMs, holdMs, enabled }) => { must(group(groupId), 'group').gate = [thresholdDb, attackMs, releaseMs, holdMs, enabled]; },
    set_pan: ({ groupId, pan, mono }) => { Object.assign(must(group(groupId), 'group'), { pan: clamp(pan, -1, 1), mono }); },
    list_group_pans: () => S.groups.map((g) => [g.id, g.pan, g.mono]),
    get_group_details: ({ groupId }) => { const g = must(group(groupId), 'group'); return copy([g.bands, g.hp, g.lp, g.comp, g.gate]); },

    list_available_plugins: () => PLUGINS,
    list_group_plugins: ({ groupId }) => must(group(groupId), 'group').plugins.map((p, i) => [i, p.type, p.enabled, p.params.map((x) => x[0]), p.params.map((x) => x[1])]),
    add_group_plugin: ({ groupId, pluginType }) => {
      const spec = must(PLUGINS.find((p) => p[0] === pluginType), 'plugin type');
      must(group(groupId), 'group').plugins.push({ type: pluginType, enabled: true, params: spec[2].map(([name, , , def]) => [name, def]) });
    },
    remove_group_plugin: ({ groupId, index }) => { must(group(groupId), 'group').plugins.splice(index, 1); },
    set_group_plugin_bypass: ({ groupId, index, bypassed }) => { must(group(groupId)?.plugins[index], 'plugin').enabled = !bypassed; },
    set_group_plugin_param: ({ groupId, index, paramName, value }) => { must(group(groupId)?.plugins[index]?.params.find((p) => p[0] === paramName), 'parameter')[1] = value; },
    reorder_group_plugin: ({ groupId, fromIndex, toIndex }) => { const list = must(group(groupId), 'group').plugins; list.splice(toIndex, 0, ...list.splice(fromIndex, 1)); },

    get_levels: () => S.groups.map((g) => {
      const { peak, compGr, gateGr } = outputLevel(g);
      return [g.id, peak * Math.min(1, 1 - g.pan), peak * Math.min(1, 1 + g.pan), compGr, gateGr];
    }),
    get_spectrum: ({ groupId }) => {
      const g = group(groupId);
      if (!g) return [];
      const heard = S.streams.filter((s) => s.group === g.id).map((s) => [streamLevel(s), s.shape]).filter(([l]) => l > 0);
      if (!heard.length) return [];
      return Array.from({ length: 24 }, (_, i) => {
        const x = i / 23; // 0 = 40 Hz, 1 = 18 kHz
        const energy = heard.reduce((sum, [l, [lo, mid, hi]]) =>
          sum + l * (lo * Math.exp(-((x - 0.12) ** 2) / 0.02) + mid * Math.exp(-((x - 0.5) ** 2) / 0.04) + hi * 0.5 * Math.exp(-((x - 0.85) ** 2) / 0.03)), 0);
        return clamp(db(energy * (0.75 + 0.25 * wob(11 + i * 1.7, i))) - 6, -60, 0);
      });
    },
    get_metrics: () => [3 + 2 * wob(0.3, 0), 1.4, 1024, 0],
    run_health_checks: () => ['ok', [
      ['PipeWire connection', 'ok', 'Connected. (This is the website demo: a pretend mixer, no real audio.)'],
      ['Groups', 'ok', `${S.groups.length} group(s), all present in the PipeWire graph.`],
      ['Audio buffer', 'ok', '1024 frames/buffer.'],
      ['Underruns', 'ok', 'None since the daemon started.'],
      ['Outputs', 'ok', `${S.outputs.length} output(s), ${S.outputs.filter((o) => o.available).length} available.`],
      ['Output routes', 'ok', 'Every configured route points at a currently-available output.'],
    ]],
    get_graph: () => {
      const edges = S.streams.map((s) => [`app:${s.id}`, `group:${s.group}`]);
      for (const g of S.groups) {
        const routes = g.routes.length ? g.routes.map((r) => r.outputId) : [S.outputs.find((o) => o.isDefault).id];
        if (g.id !== 2) routes.forEach((id) => edges.push([`group:${g.id}`, `output:${id}`]));
      }
      return [[
        ...S.streams.map((s) => [`app:${s.id}`, s.name, 'app', s.playing ? 'running' : 'idle']),
        ...S.groups.map((g) => [`group:${g.id}`, g.name, 'group', 'active']),
        ...S.outputs.map((o) => [`output:${o.id}`, o.name, 'output', o.available ? 'available' : 'unavailable']),
      ], edges];
    },

    list_outputs: () => S.outputs.map((o) => [o.id, o.name, o.desc, o.kind, o.available, o.isDefault, 48000, 2]),
    get_output_volume: ({ outputId }) => { const o = must(output(outputId), 'output'); return [o.volume, o.muted]; },
    set_output_volume: ({ outputId, volume }) => { must(output(outputId), 'output').volume = volume; return true; },
    set_output_muted: ({ outputId, muted }) => { must(output(outputId), 'output').muted = muted; return true; },
    list_group_output_routes: ({ groupId }) => routesOf(must(group(groupId), 'group').routes),
    set_group_output_route: ({ groupId, outputId, gain, enabled }) => setRoute(must(group(groupId), 'group').routes, outputId, gain, enabled),
    remove_group_output_route: ({ groupId, outputId }) => dropRoute(must(group(groupId), 'group').routes, outputId),
    set_output_synchronization: ({ groupId, outputId, syncMode, manualDelayMs }) => { Object.assign(must(group(groupId)?.routes.find((r) => r.outputId === outputId), 'route'), { sync: syncMode, delay: manualDelayMs }); },
    list_master_output_routes: () => routesOf(S.masterRoutes),
    set_master_output_route: ({ outputId, gain, enabled }) => setRoute(S.masterRoutes, outputId, gain, enabled),
    remove_master_output_route: ({ outputId }) => dropRoute(S.masterRoutes, outputId),
    set_master_output_synchronization: ({ outputId, syncMode, manualDelayMs }) => { Object.assign(must(S.masterRoutes.find((r) => r.outputId === outputId), 'route'), { sync: syncMode, delay: manualDelayMs }); },
    set_max_compensation_ms: ({ value }) => { S.maxCompMs = clamp(value, 0, 500); },
    get_max_compensation_ms: () => S.maxCompMs,
    set_preferred_default_output: ({ outputId }) => { S.preferred = outputId; },
    get_preferred_default_output: () => S.preferred,
    set_buffer_ms: ({ value }) => { S.bufferMs = clamp(value, 10, 150); },
    get_buffer_ms: () => S.bufferMs,

    set_cue_output: ({ outputId, enabled }) => { S.cueOutputs = S.cueOutputs.filter((id) => id !== outputId).concat(enabled ? [outputId] : []); },
    list_cue_outputs: () => S.cueOutputs,
    get_cue_group_id: () => 2,
    set_cue_duck_level: ({ value }) => { S.cueDuck = clamp(value, 0, 1); },
    get_cue_duck_level: () => S.cueDuck,

    list_scenes: () => S.scenes.map((s) => [s.id, s.name, '']),
    capture_current_as_scene: ({ name }) => { const id = nextId++; S.scenes.push({ id, name, state: snapshot() }); return id; },
    update_scene: ({ sceneId }) => { const s = S.scenes.find((x) => x.id === sceneId); if (s) s.state = snapshot(); return !!s; },
    duplicate_scene: ({ sceneId, newName }) => { const s = must(S.scenes.find((x) => x.id === sceneId), 'scene'); const id = nextId++; S.scenes.push({ id, name: newName, state: copy(s.state) }); return id; },
    delete_scene: ({ sceneId }) => { S.scenes = S.scenes.filter((s) => s.id !== sceneId); },
    activate_scene: ({ sceneId }) => {
      const { groups, master, masterRoutes, placement } = copy(must(S.scenes.find((s) => s.id === sceneId), 'scene').state);
      Object.assign(S, { groups, master, masterRoutes });
      placement.forEach(([id, g]) => stream(id) && (stream(id).group = group(g) ? g : 1));
    },

    list_rules: () => S.rules.map(describeRule),
    get_rule: ({ ruleId }) => { const r = S.rules.find((x) => x.id === ruleId); return r ? JSON.stringify(r) : ''; },
    save_rule: ({ json }) => {
      const rule = JSON.parse(json);
      if (!rule.actions?.length) throw 'InvalidArgs: a rule needs at least one action';
      const at = S.rules.findIndex((r) => r.id === rule.id);
      if (at >= 0) S.rules[at] = rule; else S.rules.push(Object.assign(rule, { id: nextId++ }));
      return rule.id;
    },
    set_rule_enabled: ({ ruleId, enabled }) => { must(S.rules.find((r) => r.id === ruleId), 'rule').enabled = enabled; },
    delete_rule: ({ ruleId }) => { S.rules = S.rules.filter((r) => r.id !== ruleId); },
    rule_history: () => [],

    list_ducking_rules: () => S.ducking.map((r) => [r.id, r.enabled, r.trigger, r.targets, r.reduction, r.attack, r.hold, r.release, r.threshold]),
    create_ducking_rule: ({ triggerGroup, targetGroups, reductionDb, attackMs, holdMs, releaseMs, thresholdDb }) => {
      const id = nextId++;
      S.ducking.push({ id, enabled: true, trigger: triggerGroup, targets: targetGroups, reduction: reductionDb, attack: attackMs, hold: holdMs, release: releaseMs, threshold: thresholdDb });
      return id;
    },
    set_ducking_rule_enabled: ({ ruleId, enabled }) => { must(S.ducking.find((r) => r.id === ruleId), 'ducking rule').enabled = enabled; },
    delete_ducking_rule: ({ ruleId }) => { S.ducking = S.ducking.filter((r) => r.id !== ruleId); },

    start_recording: ({ groupId, path, format }) => {
      if (!group(groupId) || !['wav16', 'wav32', 'flac'].includes(format)) return NONE;
      const id = nextId++;
      S.recordings.push({ id, groupId, path, since: now() });
      return id;
    },
    stop_recording: ({ id }) => { S.recordings = S.recordings.filter((r) => r.id !== id); },
    list_recordings: () => S.recordings.map((r) => [r.id, r.groupId, r.path, now() - r.since, Math.round((now() - r.since) * 48000)]),

    get_autostart: () => S.autostart,
    set_autostart: ({ enabled }) => { S.autostart = enabled; },
    get_quickshell_autostart: () => S.quickshell,
    set_quickshell_autostart: ({ enabled }) => { S.quickshell = enabled; },
  };

  window.__TAURI__ = {
    core: {
      // Async and by-value, like the real bridge: callers never share our state.
      invoke: async (command, args = {}) => {
        if (!api[command]) throw `demo: no such command "${command}"`;
        const result = api[command](args);
        return result === undefined ? null : copy(result);
      },
    },
  };
})();
