// The mixing desk: a made-up piece of hardware that stands in for SonicForge.
// Six group strips and a master, built from primitives and lit like a product
// photograph. Faders drag, mute buttons click, meters follow a pretend signal.
import * as THREE from 'three';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import * as tex from './textures';

const GROUPS = ['Music', 'Calls', 'Game', 'Browser', 'Video', 'Alerts'];
const STRIPS = GROUPS.length + 1; // + master
const MASTER = STRIPS - 1;

const L: tex.PanelLayout = {
  width: 7.4, depth: 6.0, strips: STRIPS,
  stripX: (i) => -3 + i,
  knobZ: [-2.3, -1.6, -0.9, -0.2],
  buttonZ: 0.3, faderNear: 2.25, faderFar: 0.75, ledX: 0.3,
};
const LEDS = 12;
const TOP = 0.085; // height of the panel's top surface in panel space
const RAKE = Math.atan(0.8 / 6.1);

export const SCENES: { name: string; levels: number[] }[] = [
  { name: 'Work', levels: [0.42, 0.8, 0.08, 0.5, 0.3, 0.62, 0.72] },
  { name: 'Game night', levels: [0.3, 0.74, 0.86, 0.18, 0.1, 0.3, 0.8] },
  { name: 'Late film', levels: [0.1, 0.28, 0.08, 0.3, 0.84, 0.14, 0.55] },
];
const START = [0.7, 0.64, 0.58, 0.46, 0.6, 0.5, 0.75];

// Camera stations, one per section of the page. `shift` slides the picture
// sideways (fraction of the viewport) to leave room for the text.
const SHOTS = [
  { pos: [-10.2, 9.6, 16.4], look: [0.2, 1.1, -0.2], shift: 0.25 },  // the whole desk
  { pos: [-3.5, 14.8, 13.4], look: [-0.2, 0.9, 0.8], shift: 0.22 },  // faders and tape
  { pos: [10.4, 5.3, 9.4], look: [0.5, 1.2, -0.8], shift: -0.3 },   // across the knobs
  { pos: [8.2, 5.6, -17.0], look: [0.0, 1.2, -4.0], shift: 0.25 },   // round the back
  { pos: [0.0, 17.6, 13.0], look: [0.0, 1.0, 0.2], shift: -0.25 },   // from above
];

// A pretend programme for each strip, so the meters have something to show.
function signal(i: number, t: number) {
  const n = (f: number, o: number) => 0.5 + 0.5 * Math.sin(t * f + o);
  switch (i) {
    case 0: return 0.5 + 0.34 * Math.pow(1 - ((t * 2.1) % 1), 3) + 0.1 * n(0.7, 1);            // music: a beat
    case 1: return THREE.MathUtils.smoothstep(n(0.45, 0) * n(0.23, 2), 0.18, 0.3) * (0.45 + 0.4 * n(9, 0) * n(5.3, 1)); // speech
    case 2: return 0.22 + 0.72 * Math.pow(n(0.9, 3) * n(1.7, 1), 2);                           // game: bursts
    case 3: return 0.18 + 0.25 * n(0.3, 5) * n(2.2, 0);
    case 4: return 0.4 + 0.3 * n(1.3, 2) * n(3.1, 4);
    default: { const p = t % 5.5; return p < 0.6 ? 0.8 * (1 - p / 0.6) : 0; }                  // alerts: a ping
  }
}
const gain = (v: number) => 1.3 * Math.pow(v, 1.5);

function sideProfile(grow = 0) {
  // Desk seen from the side as (z, y): armrest, raked panel, meter bridge, back.
  const pts = [[3.5, 0], [3.5, 0.5], [3.05, 0.62], [-3.05, 1.42], [-3.05, 1.5], [-3.55, 2.75], [-4.1, 2.75], [-4.3, 0]];
  const s = new THREE.Shape();
  pts.forEach(([z, y], i) => {
    const zz = -0.4 + (z + 0.4) * (1 + grow * 0.25);
    const yy = y === 0 ? 0 : y + grow;
    i ? s.lineTo(-zz, yy) : s.moveTo(-zz, yy);
  });
  s.closePath();
  return s;
}

function extrudeAcross(shape: THREE.Shape, width: number) {
  const g = new THREE.ExtrudeGeometry(shape, { depth: width, bevelEnabled: true, bevelSize: 0.03, bevelThickness: 0.03, bevelSegments: 3 });
  g.rotateY(Math.PI / 2);
  g.translate(-width / 2, 0, 0);
  return g;
}

export interface Desk {
  setProgress(p: number): void;
  cycleScenes(on: boolean): void;
  pause(on: boolean): void;
  onScene: (name: string) => void;
  dispose(): void;
}

export function createDesk(canvas: HTMLCanvasElement, still: boolean): Desk {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.shadowMap.enabled = true;

  const scene = new THREE.Scene();
  scene.environment = new THREE.PMREMGenerator(renderer).fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environmentIntensity = 0.55;
  const camera = new THREE.PerspectiveCamera(30, 1, 0.5, 120);

  const key = new THREE.DirectionalLight(0xfff2dc, 2.2);
  key.position.set(-6, 11, 7);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  Object.assign(key.shadow.camera, { left: -9, right: 9, top: 9, bottom: -9, near: 1, far: 30 });
  key.shadow.bias = -0.0004;
  key.shadow.radius = 5;
  scene.add(key);
  const rim = new THREE.PointLight(0xffb46b, 160, 40);
  rim.position.set(5, 7, -13);
  scene.add(rim);

  // The floor only shows the shadow; the page's own colour is the backdrop.
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(80, 80), new THREE.ShadowMaterial({ opacity: 0.42 }));
  floor.rotation.x = -Math.PI / 2;
  floor.receiveShadow = true;
  scene.add(floor);

  const desk = new THREE.Group();
  desk.position.y = 0.1;
  scene.add(desk);

  const M = {
    chassis: new THREE.MeshStandardMaterial({ color: 0x1a2124, roughness: 0.62, metalness: 0.25 }),
    black: new THREE.MeshStandardMaterial({ color: 0x121415, roughness: 0.38 }),
    rubber: new THREE.MeshStandardMaterial({ color: 0x0e0f10, roughness: 0.85 }),
    metal: new THREE.MeshStandardMaterial({ color: 0xc9ccc8, roughness: 0.28, metalness: 1 }),
    leather: new THREE.MeshStandardMaterial({ color: 0x2a1b14, roughness: 0.7 }),
    white: new THREE.MeshStandardMaterial({ color: 0xf2efe6, roughness: 0.5 }),
    enamelEdge: new THREE.MeshStandardMaterial({ color: 0xd9cfb6, roughness: 0.45 }),
  };
  const add = <T extends THREE.Object3D>(parent: THREE.Object3D, o: T, x = 0, y = 0, z = 0) => {
    o.position.set(x, y, z);
    parent.add(o);
    return o;
  };

  // Body, walnut cheeks, armrest, feet.
  add(desk, new THREE.Mesh(extrudeAcross(sideProfile(), 7.6), M.chassis));
  const wood = tex.woodTexture();
  wood.repeat.set(0.125, 0.34);
  const walnut = new THREE.MeshPhysicalMaterial({ map: wood, roughness: 0.5, clearcoat: 0.5, clearcoatRoughness: 0.35 });
  const cheek = extrudeAcross(sideProfile(0.09), 0.2);
  add(desk, new THREE.Mesh(cheek, walnut), -3.96);
  add(desk, new THREE.Mesh(cheek, walnut), 3.96);
  const arm = add(desk, new THREE.Mesh(new RoundedBoxGeometry(7.62, 0.12, 0.5, 4, 0.05), M.leather), 0, 0.61, 3.27);
  arm.rotation.x = 0.26;
  for (const x of [-3.4, 3.4]) for (const z of [-3.8, 3.0]) {
    add(desk, new THREE.Mesh(new THREE.CylinderGeometry(0.16, 0.18, 0.1, 24), M.rubber), x, -0.05, z);
  }

  // ----- The raked control panel -----
  const panel = add(desk, new THREE.Group(), 0, 1.02, 0);
  panel.rotation.x = RAKE;
  const enamel = new THREE.MeshPhysicalMaterial({ map: tex.panelTexture(L), roughness: 0.42, clearcoat: 0.35, clearcoatRoughness: 0.3 });
  add(panel, new THREE.Mesh(new THREE.BoxGeometry(L.width, 0.05, L.depth),
    [M.enamelEdge, M.enamelEdge, enamel, M.enamelEdge, M.enamelEdge, M.enamelEdge]), 0, TOP - 0.025, 0);

  const screw = new THREE.CylinderGeometry(0.05, 0.05, 0.02, 16);
  for (const x of [-3.58, 3.58]) for (const z of [-2.88, 2.88]) add(panel, new THREE.Mesh(screw, M.metal), x, TOP + 0.008, z);

  const knobBody = new THREE.LatheGeometry(
    [[0, 0], [0.27, 0], [0.27, 0.03], [0.215, 0.075], [0.2, 0.09], [0.185, 0.36], [0.165, 0.4], [0, 0.4]].map(([r, y]) => new THREE.Vector2(r, y)), 48);
  const knobCap = new THREE.CylinderGeometry(0.15, 0.15, 0.02, 40);
  const knobLine = new THREE.BoxGeometry(0.028, 0.012, 0.2);
  const capColours = [0x8f9692, 0x2f5d8a, 0xd9a13b, 0xc8402a].map((c) => new THREE.MeshStandardMaterial({ color: c, roughness: 0.45 }));

  const capShape = new THREE.Shape();
  capShape.moveTo(-0.21, 0); capShape.lineTo(0.21, 0); capShape.lineTo(0.21, 0.16); capShape.lineTo(0.17, 0.2);
  capShape.quadraticCurveTo(0, 0.1, -0.17, 0.2); capShape.lineTo(-0.21, 0.16); capShape.closePath();
  const capGeo = new THREE.ExtrudeGeometry(capShape, { depth: 0.22, bevelEnabled: true, bevelSize: 0.012, bevelThickness: 0.012, bevelSegments: 2 });
  capGeo.rotateY(Math.PI / 2);
  capGeo.translate(-0.11, 0, 0);
  const capDark = new THREE.MeshStandardMaterial({ color: 0x1b1d1e, roughness: 0.32 });
  const capRed = new THREE.MeshStandardMaterial({ color: 0xc8402a, roughness: 0.38 });
  const buttonGeo = new RoundedBoxGeometry(0.2, 0.09, 0.2, 3, 0.03);

  const strips = Array.from({ length: STRIPS }, (_, i) => {
    const x = L.stripX(i);
    const master = i === MASTER;
    const knobs = master ? 2 : 4;
    for (let k = 0; k < knobs; k++) {
      const knob = add(panel, new THREE.Group(), x, TOP, L.knobZ[k]);
      knob.add(new THREE.Mesh(knobBody, M.black));
      add(knob, new THREE.Mesh(knobCap, capColours[master ? 3 : k]), 0, 0.41, 0);
      add(knob, new THREE.Mesh(knobLine, M.white), 0, 0.421, -0.06);
      knob.scale.setScalar(0.84);
      knob.rotation.y = (((i * 7 + k * 13) % 11) / 10 - 0.5) * 3.6; // each one left somewhere different
    }

    const mute = add(panel, new THREE.Mesh(buttonGeo, new THREE.MeshStandardMaterial({ color: 0x5a1a12, emissive: 0xff3a1c, emissiveIntensity: 0, roughness: 0.3 })), x - 0.2, TOP + 0.03, L.buttonZ);
    add(panel, new THREE.Mesh(buttonGeo, new THREE.MeshStandardMaterial({ color: 0x5a4212, emissive: 0xffa21c, emissiveIntensity: i === 1 ? 1.6 : 0, roughness: 0.3 })), x + 0.2, TOP + 0.03, L.buttonZ);

    const cap = add(panel, new THREE.Group(), x - 0.08, TOP, 0);
    cap.add(new THREE.Mesh(capGeo, master ? capRed : capDark));
    add(cap, new THREE.Mesh(new THREE.BoxGeometry(0.245, 0.006, 0.022), M.white), 0, 0.152, 0);

    return { mute, cap, value: START[i], target: START[i], muted: false, level: 0, peak: 0 };
  });
  let duck = 1;

  // Level LEDs: one instanced mesh, coloured per segment every frame.
  const leds = add(panel, new THREE.InstancedMesh(new THREE.BoxGeometry(0.085, 0.03, 0.082), new THREE.MeshBasicMaterial({ toneMapped: false }), STRIPS * LEDS));
  const ledOn = [0x5dff7a, 0xffc93c, 0xff4326].map((c) => new THREE.Color(c));
  const ledOff = [0x0d2a14, 0x2e2409, 0x300d07].map((c) => new THREE.Color(c));
  const ledBand = (k: number) => (k < 7 ? 0 : k < 10 ? 1 : 2);
  const m4 = new THREE.Matrix4();
  for (let i = 0; i < STRIPS; i++) for (let k = 0; k < LEDS; k++) {
    const z = L.faderNear - ((k + 0.5) / LEDS) * (L.faderNear - L.faderFar);
    leds.setMatrixAt(i * LEDS + k, m4.makeTranslation(L.stripX(i) + L.ledX, TOP + 0.012, z));
  }

  // ----- Meter bridge -----
  const bridge = add(desk, new THREE.Group(), 0, 2.125, -3.3);
  bridge.rotation.x = -0.3805;
  const BW = 7.4, BH = 1.3;
  add(bridge, new THREE.Mesh(new THREE.BoxGeometry(BW, BH, 0.04),
    [M.chassis, M.chassis, M.chassis, M.chassis, new THREE.MeshStandardMaterial({ map: tex.bridgeTexture(BW, BH), roughness: 0.45, metalness: 0.6 }), M.chassis]), 0, 0, 0.05);
  const face = tex.vuTexture();
  const faceMat = new THREE.MeshStandardMaterial({ map: face, emissiveMap: face, emissive: 0xffc27a, emissiveIntensity: 0.85, roughness: 0.7 });
  const glass = new THREE.MeshPhysicalMaterial({ color: 0xffffff, transparent: true, opacity: 0.12, roughness: 0.04, clearcoat: 1 });
  const needleGeo = new THREE.BoxGeometry(0.012, 0.47, 0.004).translate(0, 0.235, 0);
  const needles = strips.map((_, i) => {
    const vu = add(bridge, new THREE.Group(), L.stripX(i), 0.13, 0.07);
    vu.add(new THREE.Mesh(new RoundedBoxGeometry(0.9, 0.72, 0.12, 3, 0.03), M.black));
    add(vu, new THREE.Mesh(new THREE.PlaneGeometry(0.8, 0.6), faceMat), 0, 0, 0.062);
    add(vu, new THREE.Mesh(new THREE.BoxGeometry(0.8, 0.11, 0.03), M.black), 0, -0.245, 0.08);
    add(vu, new THREE.Mesh(new THREE.PlaneGeometry(0.8, 0.6), glass), 0, 0, 0.1);
    return add(vu, new THREE.Mesh(needleGeo, M.black), 0, -0.36, 0.072);
  });

  // ----- Console tape -----
  const tape = add(panel, new THREE.Mesh(new THREE.PlaneGeometry(7.1, 0.34),
    new THREE.MeshStandardMaterial({ map: tex.tapeTexture([...GROUPS, 'Master'], 7.1, 0.34), roughness: 0.9, alphaTest: 0.5 })), 0.02, TOP + 0.004, 2.64);
  tape.rotation.set(-Math.PI / 2, 0, 0.006);

  // ----- Rear panel: outputs and their cables -----
  const rear = add(desk, new THREE.Group(), 0, 1.4, -4.235);
  rear.lookAt(0, 1.5 + 0.0725, -4.235 - 0.997); // face out of the back, leaning with the chassis
  const outputs = ['SPEAKERS', 'HEADPHONES', 'DESK MONITOR', 'USB DAC'];
  const jackX = outputs.flatMap((_, i) => [-2.55 + i * 1.7 - 0.28, -2.55 + i * 1.7 + 0.28]);
  add(rear, new THREE.Mesh(new THREE.PlaneGeometry(7.2, 1.1), new THREE.MeshStandardMaterial({ map: tex.rearTexture(outputs, jackX, 7.2, 1.1), roughness: 0.6, metalness: 0.3 })), 0, 0, 0.002);
  const nut = new THREE.CylinderGeometry(0.11, 0.11, 0.05, 6).rotateX(Math.PI / 2);
  const hole = new THREE.CylinderGeometry(0.055, 0.055, 0.052, 20).rotateX(Math.PI / 2);
  const plug = new THREE.CylinderGeometry(0.065, 0.075, 0.42, 20).rotateX(Math.PI / 2);
  const band = new THREE.CylinderGeometry(0.078, 0.078, 0.05, 20).rotateX(Math.PI / 2);
  const bands = [0xc8402a, 0xc8402a, 0xd9a13b, 0xd9a13b, 0, 0, 0x2f5d8a, 0].map((c) => c && new THREE.MeshStandardMaterial({ color: c, roughness: 0.5 }));
  // Cables fade out along their length, so they trail off instead of ending.
  const cableMat = new THREE.MeshStandardMaterial({ color: 0x0e0f10, roughness: 0.85, transparent: true, alphaMap: tex.fadeTexture(0.1, 0.24) });
  desk.updateMatrixWorld(true);
  jackX.forEach((x, i) => {
    add(rear, new THREE.Mesh(nut, M.metal), x, -0.06, 0.025);
    add(rear, new THREE.Mesh(hole, M.rubber), x, -0.06, 0.03);
    const colour = bands[i];
    if (!colour) return; // nothing plugged in here
    add(rear, new THREE.Mesh(plug, M.black), x, -0.06, 0.27);
    add(rear, new THREE.Mesh(band, colour), x, -0.06, 0.12);
    const from = rear.localToWorld(new THREE.Vector3(x, -0.06, 0.46));
    // Down to the floor, then a meandering run off into the dark behind the desk.
    const run = new THREE.CatmullRomCurve3([
      from,
      from.clone().add(new THREE.Vector3(0, -0.12, -0.5)),
      new THREE.Vector3(from.x + 0.1, 0.3, from.z - 1.5),
      new THREE.Vector3(from.x + 0.4 - i * 0.1, 0.05, from.z - 2.5),
      new THREE.Vector3(from.x * 0.8 + 0.9, 0.05, from.z - 5),
      new THREE.Vector3(from.x * 0.6 - 0.6 + i * 0.15, 0.05, from.z - 9),
      new THREE.Vector3(from.x * 0.4 + 2.2 + i * 0.2, 0.05, from.z - 15),
      new THREE.Vector3(from.x * 0.3 + 0.5 + i * 0.3, 0.05, from.z - 24),
      new THREE.Vector3(from.x * 0.2 + 5 + i * 0.4, 0.05, from.z - 40),
    ]);
    const cable = new THREE.Mesh(new THREE.TubeGeometry(run, 200, 0.038, 8), cableMat);
    cable.userData.noShadow = true;
    scene.add(cable);
  });

  scene.traverse((o) => {
    if (o instanceof THREE.Mesh && o !== floor) {
      o.castShadow = o.material !== glass && o !== tape && !o.userData.noShadow;
      o.receiveShadow = true;
    }
  });

  // ----- Camera -----
  const posCurve = new THREE.CatmullRomCurve3(SHOTS.map((s) => new THREE.Vector3(...s.pos)));
  const lookCurve = new THREE.CatmullRomCurve3(SHOTS.map((s) => new THREE.Vector3(...s.look)));
  let goal = 0, at = 0, width = 1, height = 1;
  const look = new THREE.Vector3();

  function frameCamera() {
    const u = at / (SHOTS.length - 1);
    lookCurve.getPoint(u, look);
    posCurve.getPoint(u, camera.position);
    const i = Math.min(Math.floor(at), SHOTS.length - 2);
    const shift = THREE.MathUtils.lerp(SHOTS[i].shift, SHOTS[i + 1].shift, at - i);
    // On a tall, narrow screen the text sits below, so centre the desk, lift it and step back.
    const narrow = width / height < 1.05;
    const back = narrow ? THREE.MathUtils.clamp(1.15 / (width / height), 1, 2.3) : 1;
    camera.position.sub(look).multiplyScalar(back).add(look);
    camera.lookAt(look);
    camera.setViewOffset(width, height, narrow ? 0 : -shift * width, narrow ? 0.24 * height : 0, width, height);
  }

  function resize() {
    width = canvas.clientWidth;
    height = canvas.clientHeight;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  }
  const ro = new ResizeObserver(resize);
  ro.observe(canvas);
  resize();

  // ----- Hands on: drag a fader, click a mute -----
  const ray = new THREE.Raycaster();
  const ptr = new THREE.Vector2();
  const plane = new THREE.Plane();
  const hit = new THREE.Vector3();
  let dragging = -1, grab = 0;
  const aim = (e: PointerEvent) => {
    const r = canvas.getBoundingClientRect();
    ptr.set(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(ptr, camera);
  };
  const under = (objs: THREE.Object3D[]) => {
    const h = ray.intersectObjects(objs, true)[0];
    return h ? objs.findIndex((o) => o === h.object || o === h.object.parent) : -1;
  };
  const panelZ = () => {
    plane.setFromNormalAndCoplanarPoint(new THREE.Vector3(0, 1, 0).transformDirection(panel.matrixWorld), panel.getWorldPosition(hit));
    return ray.ray.intersectPlane(plane, hit) ? panel.worldToLocal(hit).z : null;
  };
  const caps = strips.map((s) => s.cap);
  const mutes = strips.map((s) => s.mute);

  // ponytail: mouse and pen only. A touch drag on the canvas has to stay a page scroll; add a long-press gesture if touch control is wanted.
  function down(e: PointerEvent) {
    if (e.pointerType === 'touch') return;
    aim(e);
    const c = under(caps);
    if (c >= 0) {
      dragging = c;
      grab = (panelZ() ?? 0) - strips[c].cap.position.z;
      canvas.setPointerCapture(e.pointerId);
      canvas.style.cursor = 'grabbing';
      e.preventDefault();
      return;
    }
    const m = under(mutes);
    if (m >= 0) strips[m].muted = !strips[m].muted;
  }
  function move(e: PointerEvent) {
    if (e.pointerType === 'touch') return;
    aim(e);
    if (dragging < 0) {
      canvas.style.cursor = under(caps) >= 0 ? 'grab' : under(mutes) >= 0 ? 'pointer' : '';
      return;
    }
    const z = panelZ();
    if (z === null) return;
    const s = strips[dragging];
    s.value = s.target = THREE.MathUtils.clamp((L.faderNear - (z - grab)) / (L.faderNear - L.faderFar), 0, 1);
  }
  function up() {
    dragging = -1;
    canvas.style.cursor = '';
  }
  canvas.addEventListener('pointerdown', down);
  canvas.addEventListener('pointermove', move);
  canvas.addEventListener('pointerup', up);
  canvas.addEventListener('pointercancel', up);

  // ----- Scene recall -----
  let cycling = false, sceneAt = -1, nextScene = 0;
  const api: Desk = {
    setProgress(p) { goal = THREE.MathUtils.clamp(p, 0, SHOTS.length - 1); },
    cycleScenes(on) {
      if (on === cycling) return;
      cycling = on;
      nextScene = 0;
      if (!on) strips.forEach((s, i) => (s.target = START[i]));
    },
    pause(on) { paused = on; },
    onScene: () => {},
    dispose() {
      cancelAnimationFrame(raf);
      ro.disconnect();
      renderer.dispose();
    },
  };

  // ----- Frame loop -----
  const timer = new THREE.Timer();
  let raf = 0, t = 1.3, paused = false;
  function frame() {
    raf = requestAnimationFrame(frame);
    timer.update();
    const dt = Math.min(timer.getDelta(), 0.05);
    if (paused || document.hidden) return;
    if (!still) t += dt;
    const ease = (rate: number) => (still ? 1 : 1 - Math.exp(-dt * rate));

    at += (goal - at) * ease(4);
    frameCamera();

    if (cycling && dragging < 0 && (nextScene -= dt) <= 0) {
      sceneAt = (sceneAt + 1) % SCENES.length;
      SCENES[sceneAt].levels.forEach((v, i) => (strips[i].target = v));
      api.onScene(SCENES[sceneAt].name);
      nextScene = 3.6;
    }

    // Ducking, as the real mixer does it: Music drops while Calls is heard.
    const speech = strips[1].muted ? 0 : signal(1, t) * gain(strips[1].value);
    duck += ((speech > 0.18 ? 0.35 : 1) - duck) * ease(speech > 0.18 ? 14 : 2.5);

    let sum = 0;
    strips.forEach((s, i) => {
      s.value += (s.target - s.value) * ease(5); // motorised glide
      s.cap.position.z = L.faderNear - s.value * (L.faderNear - L.faderFar);
      (s.mute.material as THREE.MeshStandardMaterial).emissiveIntensity = s.muted ? 2.2 : 0;

      let out: number;
      if (i === MASTER) out = Math.min(1.15, 0.62 * Math.sqrt(sum)) * gain(s.value);
      else {
        out = s.muted ? 0 : signal(i, t) * gain(s.value) * (i === 0 ? duck : 1);
        sum += out * out;
      }
      s.level += (out - s.level) * ease(7);                 // VU ballistics: slow and heavy
      s.peak = Math.max(out, s.peak - dt * 0.9);            // LEDs: instant up, slow down
      needles[i].rotation.z = THREE.MathUtils.lerp(0.82, -0.82, Math.min(s.level * 1.9, 1.08) / 1.08);
      const lit = Math.round(Math.min(s.peak * 1.8, 1) * LEDS);
      for (let k = 0; k < LEDS; k++) leds.setColorAt(i * LEDS + k, (k < lit ? ledOn : ledOff)[ledBand(k)]);
    });
    leds.instanceColor!.needsUpdate = true;

    renderer.render(scene, camera);
  }
  frame();
  return api;
}
