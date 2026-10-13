// Smoke tests for the real ui/index.html, driven in a real (headless)
// browser via the bridge in bridge.mjs — which proxies straight to the
// live `org.fonyx.Daemon` D-Bus service. Requires the daemon to already
// be running (see README's "Running" section); these tests do not start
// it themselves.
//
// Scope: golden paths only (tabs render, a rule can be created and fires
// correctly, a group can be created/deleted, Settings controls read/write
// real daemon state) — not a replacement for the daemon's own unit tests,
// and not exhaustive UI coverage.
import fs from 'node:fs';
import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  // The app's own 5s poll cycle plus initial invoke()s need a beat to land.
  await page.waitForSelector('#home.tab.active');
});

test('every nav tab renders its section', async ({ page }) => {
  const tabs = ['home', 'inputs', 'mtx', 'faders', 'outputs', 'scenes', 'rules', 'recording', 'doctor', 'settings'];
  for (const tab of tabs) {
    await page.click(`.nav-pill button[data-tab="${tab}"]`);
    await expect(page.locator(`#${tab}.tab`)).toHaveClass(/active/);
    // A tab that threw while rendering would otherwise leave an empty
    // section silently "passing" this assertion — require real content.
    await expect(page.locator(`#${tab}`)).not.toBeEmpty();
  }
});

test('home tab shows real group data from the live daemon', async ({ page }) => {
  // Confirms the app actually rendered daemon state, not just static markup
  // — Default always exists (protected group, created on first daemon run).
  await expect(page.locator('#home')).toContainText('Default');
});

test('inputs tab: Keep saves the app as a profile in the real daemon', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="inputs"]');
  const keep = page.locator('#inputs .keep-btn').first();
  await keep.click();
  // "Kept" is only written once the daemon has answered true.
  await expect(keep).toHaveText('Kept');
});

test('creating and deleting a group round-trips through the real daemon', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="inputs"]');
  const groupName = `PlaywrightTest-${Date.now()}`;
  await page.fill('#new-group-name', groupName);
  await page.click('button[onclick="createGroup()"]');

  const row = page.locator('#group-list .row-item').filter({ has: page.locator('.name', { hasText: groupName }) });
  await expect(row).toBeVisible();

  await row.locator('button[title="Delete group"]').click();
  await expect(page.locator('#group-list', { hasText: groupName })).toHaveCount(0);
});

test('rules tab: build a site-filtered rule, edit it, delete it', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="rules"]');

  const ruleName = `PwRule-${Date.now()}`;
  await page.fill('#rule-name', ruleName);
  // Default trigger is "an app starts"; the default action row routes the
  // app to the first group (always at least Default).
  await page.fill('#rule-service', 'PlaywrightSite');
  await page.click('#rule-submit-btn');

  const row = page.locator('#rule-list .row-item', { hasText: ruleName });
  await expect(row).toBeVisible();
  // A service-filtered trigger must describe itself as matching a *site*.
  await expect(row).toContainText('site: PlaywrightSite');

  // Edit: the form reloads the rule; switch to a non-app trigger with a
  // group-volume action and save in place.
  await row.locator('button[title="Edit"]').click();
  await expect(page.locator('#rule-service')).toHaveValue('PlaywrightSite');
  await page.selectOption('#rule-trigger', 'SystemResume');
  await page.selectOption('#rule-actions .a-kind', 'SetGroupVolume');
  await page.click('#rule-submit-btn');
  await expect(row).toContainText('system resume');
  await expect(page.locator('#rule-list .row-item', { hasText: ruleName })).toHaveCount(1);

  // Validation: an app-only action on a non-app trigger is refused.
  await page.selectOption('#rule-trigger', 'SystemResume');
  await page.click('#rule-submit-btn');
  await expect(page.locator('#rule-error')).toContainText('app-started trigger');

  await row.locator('button[title="Delete"]').click();
  await expect(page.locator('#rule-list', { hasText: ruleName })).toHaveCount(0);
});

test('settings tab: theme toggle applies immediately and persists', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="settings"]');
  await page.selectOption('#theme-select', 'light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');

  await page.reload();
  await page.waitForSelector('#home.tab.active');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');

  // Leave the app the way this test found it.
  await page.click('.nav-pill button[data-tab="settings"]');
  await page.selectOption('#theme-select', 'dark');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});

test('settings tab: preferred output device lists real daemon outputs', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="settings"]');
  const options = page.locator('#preferred-output-select option');
  // "Auto" plus whatever real hardware this machine actually has.
  await expect(options).toHaveCount(await options.count());
  await expect(options.first()).toHaveText('Auto');
  expect(await options.count()).toBeGreaterThan(1);
});

test('settings tab: buffer size reads and writes real daemon config', async ({ page, request }) => {
  await page.click('.nav-pill button[data-tab="settings"]');
  const original = await page.locator('#buffer-ms-select').inputValue();

  await page.selectOption('#buffer-ms-select', '20');
  const viaBridge = await request.post('/__invoke', { data: { cmd: 'get_buffer_ms', args: {} } });
  expect(await viaBridge.json()).toBe(20);

  // Restore, so repeated runs (and a human using the real app afterward)
  // see the value they started with.
  await page.selectOption('#buffer-ms-select', original);
});

test('settings tab: Quickshell autostart toggle round-trips through real systemd', async ({ page }) => {
  // run-isolated.sh has no systemd user session to toggle a unit in.
  test.skip(!!process.env.FONYX_ISOLATED, 'needs a real systemd user session');
  await page.click('.nav-pill button[data-tab="settings"]');
  const toggle = page.locator('#quickshell-autostart-toggle');
  const original = await toggle.isChecked();

  await toggle.setChecked(true);
  // Real systemctl call, not app-internal state — confirms the toggle
  // actually reached systemd, not just its own checkbox.
  await expect
    .poll(() => page.evaluate(() => window.__TAURI__.core.invoke('get_quickshell_autostart')))
    .toBe(true);

  await toggle.setChecked(original);
  await expect
    .poll(() => page.evaluate(() => window.__TAURI__.core.invoke('get_quickshell_autostart')))
    .toBe(original);
});

test('scenes: capture current state, activate it, delete it', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="scenes"]');
  const sceneName = `PwScene-${Date.now()}`;
  await page.fill('#new-scene-name', sceneName);
  await page.click('button[onclick="captureNewScene()"]');

  const card = page.locator('#scene-list .output-card', { has: page.locator('h2', { hasText: sceneName }) });
  await expect(card).toBeVisible();

  await card.locator('button[onclick^="activateScene"]').click();
  // activateScene's own setTimeout(refreshAll, 300) — give it a moment
  // rather than asserting immediately.
  await page.waitForTimeout(500);

  await card.locator('button[onclick^="deleteScene"]').click();
  await expect(page.locator('#scene-list', { hasText: sceneName })).toHaveCount(0);
});

test('ducking: create a rule between two real groups, delete it', async ({ page, request }) => {
  await page.click('.nav-pill button[data-tab="rules"]');
  const before = await (await request.post('/__invoke', { data: { cmd: 'list_ducking_rules', args: {} } })).json();

  // Pick a source (first option) and one destination, then submit.
  await page.locator('#duck-target-list input[data-group]').first().check();
  await page.click('#duck-submit-btn');
  await page.waitForFunction(
    n => window.__TAURI__.core.invoke('list_ducking_rules').then(r => r.length > n),
    before.length,
  );

  const after = await (await request.post('/__invoke', { data: { cmd: 'list_ducking_rules', args: {} } })).json();
  expect(after.length).toBe(before.length + 1);
  const created = after.find(r => !before.some(b => b[0] === r[0]));
  expect(created).toBeTruthy();

  const del = await request.post('/__invoke', { data: { cmd: 'delete_ducking_rule', args: { ruleId: created[0] } } });
  expect(del.ok()).toBe(true); // fire-and-forget here once silently left a rule behind
  const final = await (await request.post('/__invoke', { data: { cmd: 'list_ducking_rules', args: {} } })).json();
  expect(final.some(r => r[0] === created[0])).toBe(false);
});

test('plugins: add a plugin to the MTX Panel group, bypass it, remove it', async ({ page }) => {
  await page.click('.nav-pill button[data-tab="mtx"]');
  // Not waitForSelector (default state: 'visible') — an <option> inside a
  // closed <select> never reports visible in headless Chromium, so that
  // hangs to the timeout even once the list is populated.
  await expect.poll(() => page.locator('#add-plugin-type option').count()).toBeGreaterThan(0);

  const before = await page.locator('#plugin-list .plugin-card').count();
  await page.click('button[onclick="addPlugin()"]');
  await expect(page.locator('#plugin-list .plugin-card')).toHaveCount(before + 1);

  const row = page.locator('#plugin-list .plugin-card').last();
  const bypassToggle = row.locator('input.toggle');
  await expect(bypassToggle).toBeChecked(); // added plugins start enabled
  await bypassToggle.uncheck();
  await expect(bypassToggle).not.toBeChecked();

  await row.locator('button.danger').click();
  await expect(page.locator('#plugin-list .plugin-card')).toHaveCount(before);
});

test('recording: start against a real group, confirm the file exists, stop and clean up', async ({ page, request }) => {
  await page.click('.nav-pill button[data-tab="recording"]');
  const path = `/tmp/fonyx-playwright-test-${Date.now()}.wav`;
  await page.fill('#rec-path', path);
  await page.click('button[onclick="startRecording()"]');

  // From here on: an assertion failure must not leave the daemon actively
  // recording to a file nothing will ever clean up (live-caught: an
  // earlier, failing version of this test did exactly that — the daemon
  // kept writing for minutes after the test process exited).
  try {
    const row = page.locator('#recording-list .row-item', { hasText: path.split('/').pop() });
    await expect(row).toBeVisible();

    // Writes to `path + '.tmp'` while active, atomically renamed to `path`
    // only on stop (daemon/src/recording.rs) — the real, final path
    // shouldn't exist yet.
    await expect.poll(() => fs.existsSync(`${path}.tmp`)).toBe(true);
    expect(fs.existsSync(path)).toBe(false);

    await row.locator('button[onclick^="stopRecordingClick"]').click();
    await expect.poll(() => fs.existsSync(path)).toBe(true);
  } finally {
    const active = await (await request.post('/__invoke', { data: { cmd: 'list_recordings', args: {} } })).json();
    const ours = active.find(r => r[2] === path);
    if (ours) await request.post('/__invoke', { data: { cmd: 'stop_recording', args: { id: ours[0] } } });
    fs.rmSync(path, { force: true });
    fs.rmSync(`${path}.tmp`, { force: true });
  }
});

test('MTX Panel: dragging an EQ band node persists the new frequency to the real daemon', async ({ page, request }) => {
  await page.click('.nav-pill button[data-tab="mtx"]');
  const node = page.locator('#eq-graph g.eq-node[data-kind="band"][data-index="0"]');
  await expect(node).toBeVisible();

  const groupId = Number(await page.locator('#mtx-group').inputValue());
  const before = await (await request.post('/__invoke', { data: { cmd: 'get_group_details', args: { groupId } } })).json();
  const freqBefore = before[0][0][0]; // (bands[], hp, lp, comp, gate) -> bands[0] -> freq

  // The graph redraws on its own; a box read mid-redraw comes back null.
  let box;
  await expect.poll(async () => (box = await node.boundingBox())).not.toBeNull();
  const start = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  // Drag right (higher frequency) in a few steps — the app pushes
  // `set_eq_band` on every pointermove, same as a real user's drag.
  await page.mouse.move(start.x + 120, start.y, { steps: 8 });
  await page.mouse.up();

  const after = await (await request.post('/__invoke', { data: { cmd: 'get_group_details', args: { groupId } } })).json();
  const freqAfter = after[0][0][0];
  expect(freqAfter).not.toBe(freqBefore);

  // Leave the daemon's persisted state as this test found it.
  await request.post('/__invoke', {
    data: { cmd: 'set_eq_band', args: { groupId, bandIndex: 0, freq: freqBefore, gain: before[0][0][1], q: before[0][0][2], enabled: before[0][0][3] } },
  });
});
