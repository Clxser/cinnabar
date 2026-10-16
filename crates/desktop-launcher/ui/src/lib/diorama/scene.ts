import {
  ACESFilmicToneMapping,
  AdditiveBlending,
  BufferAttribute,
  BufferGeometry,
  CanvasTexture,
  Color,
  DirectionalLight,
  Fog,
  Group,
  HemisphereLight,
  Mesh,
  MeshStandardMaterial,
  NearestFilter,
  PCFShadowMap,
  PerspectiveCamera,
  PlaneGeometry,
  PMREMGenerator,
  Points,
  PointsMaterial,
  PointLight,
  Scene,
  ShaderMaterial,
  SRGBColorSpace,
  Vector2,
  Vector3,
  WebGLRenderer,
} from 'three';
import { EffectComposer } from 'three/examples/jsm/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/examples/jsm/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/examples/jsm/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/examples/jsm/postprocessing/OutputPass.js';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { buildAtlas, mockSkin } from './textures';
import { buildMesh, buildRock, buildWorld } from './world';
import { buildPlayer } from './player';
import { MOODS, type Mood } from './mood';

export interface DioramaOptions {
  mood: Mood;
  frame: Frame;
  /** Camera azimuth around the island, in radians. */
  orbit?: number;
  /** Render one still frame and never animate. */
  still: boolean;
  skin?: HTMLCanvasElement;
}

/** Where the island sits in the canvas: offsets as fractions of its size, and a zoom factor. */
export interface Frame {
  x: number;
  y: number;
  zoom: number;
}

/** Foreground perch placement: distance toward the camera, sideways and vertical offsets. */
const FOREGROUND = { near: 19, side: -3.4, lift: -0.6 };

export interface Diorama {
  /** Fraction of the vein that glows, 0..1. */
  setRefined(fraction: number): void;
  setMood(mood: Mood): void;
  setFrame(frame: Frame): void;
  /** Stops rendering (game running, window hidden); resuming redraws immediately. */
  setPaused(paused: boolean): void;
  pointer(x: number, y: number): void;
  resize(): void;
  dispose(): void;
}

// The camera looks down on the island, so the backdrop is graded in screen space: sky at the top,
// the horizon band low in the frame, and the void beneath the island darkening toward the bottom.
const SKY_VERTEX = /* glsl */ `
  void main() {
    gl_Position = vec4(position.xy, 1.0, 1.0);
  }`;

const SKY_FRAGMENT = /* glsl */ `
  uniform vec3 top;
  uniform vec3 horizon;
  uniform vec3 glow;
  uniform vec3 sunDir;
  uniform vec2 resolution;
  void main() {
    vec2 p = gl_FragCoord.xy / resolution;
    float up = smoothstep(0.18, 1.0, p.y);
    vec3 c = mix(horizon, top, pow(up, 0.8));
    c = mix(c, top * 0.55, smoothstep(0.3, 0.0, p.y));
    vec2 sun = vec2(sunDir.x, sunDir.y);
    float d = distance(vec2(p.x * resolution.x / resolution.y, p.y), vec2(sun.x * resolution.x / resolution.y, sun.y));
    c += glow * exp(-d * d * 9.0) * 0.55;
    gl_FragColor = vec4(c, 1.0);
  }`;

export function createDiorama(canvas: HTMLCanvasElement, options: DioramaOptions): Diorama {
  const renderer = new WebGLRenderer({ canvas, antialias: true, alpha: false, powerPreference: 'high-performance', failIfMajorPerformanceCaveat: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 1.75));
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.toneMapping = ACESFilmicToneMapping;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = PCFShadowMap;

  const scene = new Scene();
  const camera = new PerspectiveCamera(24, 1, 0.5, 400);
  const pmrem = new PMREMGenerator(renderer);
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environmentIntensity = 0.25;

  // Sky dome and fog share the mood's horizon so the island sits in air, not on a backdrop.
  const skyMat = new ShaderMaterial({
    vertexShader: SKY_VERTEX,
    fragmentShader: SKY_FRAGMENT,
    depthWrite: false,
    depthTest: false,
    uniforms: {
      top: { value: new Color() },
      horizon: { value: new Color() },
      glow: { value: new Color() },
      sunDir: { value: new Vector3() },
      resolution: { value: new Vector2(1, 1) },
    },
  });
  const sky = new Mesh(new PlaneGeometry(2, 2), skyMat);
  sky.frustumCulled = false;
  sky.renderOrder = -1;
  scene.add(sky);
  scene.fog = new Fog(0x000000, 60, 160);

  const atlas = buildAtlas();
  const tex = (c: HTMLCanvasElement, srgb: boolean) => {
    const t = new CanvasTexture(c);
    t.magFilter = NearestFilter;
    t.minFilter = NearestFilter;
    t.generateMipmaps = false;
    if (srgb) t.colorSpace = SRGBColorSpace;
    return t;
  };

  const world = buildWorld();
  const terrainMat = new MeshStandardMaterial({
    map: tex(atlas.color, true),
    emissiveMap: tex(atlas.glow, true),
    emissive: new Color(1, 1, 1),
    vertexColors: true,
    roughness: 0.92,
    metalness: 0,
  });
  // Each ore voxel glows only once the refine front has passed its index in the vein.
  const refine = { value: 1 };
  const glowGain = { value: 2.4 };
  terrainMat.onBeforeCompile = (shader) => {
    shader.uniforms.refined = refine;
    shader.uniforms.glowGain = glowGain;
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nattribute float orePos;\nvarying float vOre;')
      .replace('#include <begin_vertex>', '#include <begin_vertex>\nvOre = orePos;');
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', '#include <common>\nuniform float refined;\nuniform float glowGain;\nvarying float vOre;')
      .replace('#include <emissivemap_fragment>', '#include <emissivemap_fragment>\nfloat lit = vOre < 0.0 ? 0.0 : step(vOre, refined);\ntotalEmissiveRadiance *= lit * glowGain;');
  };
  const terrain = new Mesh(buildMesh(world, atlas), terrainMat);
  terrain.castShadow = true;
  terrain.receiveShadow = true;
  scene.add(terrain);

  const half = world.size / 2;

  // Chunks drifting at other depths give the camera's slow orbit something to parallax against.
  const rocks = [
    { at: [-9.2, 1.5, -0.1], s: 0.5, seed: 3 },
    { at: [5.9, -2, -10.1], s: 0.6, seed: 5 },
    { at: [-8.4, 3.5, -16.5], s: 0.5, seed: 8 },
  ].map(({ at, s: scale, seed }) => {
    const m = new Mesh(buildMesh(buildRock(seed), atlas), terrainMat);
    m.position.set(at[0]!, at[1]!, at[2]!);
    m.scale.setScalar(scale);
    m.castShadow = true;
    m.receiveShadow = true;
    scene.add(m);
    return { mesh: m, y: at[1]!, phase: seed };
  });
  // Liquid mercury pooled in the basin: a mirror-bright metal surface.
  const pool = new Mesh(
    new PlaneGeometry(2, 2),
    new MeshStandardMaterial({ color: 0xd9dee5, metalness: 1, roughness: 0.08, envMapIntensity: 1.6 }),
  );
  pool.rotation.x = -Math.PI / 2;
  pool.position.set(world.pool[0] + 1 - half, world.top(world.pool[0], world.pool[1]) + 1 - 0.18, world.pool[1] + 1 - half);
  pool.receiveShadow = true;
  scene.add(pool);

  // The player stands on their own stone in the foreground: nearer the camera than the island, so
  // perspective makes them the scene's figure while the island stays the backdrop.
  const player = buildPlayer(options.skin ?? mockSkin());
  const orbit = options.orbit ?? 0.7;
  const toCamera = new Vector3(Math.sin(orbit) * 0.96, 0.27, Math.cos(orbit) * 0.96);
  const right = new Vector3(Math.cos(orbit), 0, -Math.sin(orbit));
  const perch = new Mesh(buildMesh(buildRock(21), atlas), terrainMat);
  perch.castShadow = true;
  perch.receiveShadow = true;
  // A small stone under the feet; the player keeps full size.
  perch.scale.set(0.42, 0.5, 0.42);
  perch.position.set(-0.84, 0, -0.84);
  const foreground = new Group();
  foreground.add(perch);
  player.root.position.set(0, 0.5, 0);
  player.root.rotation.y = orbit + 0.35;
  foreground.add(player.root);
  foreground.position.copy(toCamera.multiplyScalar(FOREGROUND.near)).addScaledVector(right, FOREGROUND.side).add(new Vector3(0, FOREGROUND.lift, 0));
  scene.add(foreground);

  // Warm light pooled above the vein so nearby faces pick up its colour.
  const oreLight = new PointLight(0xff5a3c, 0, 9, 1.6);
  const mid = world.ore[Math.floor(world.ore.length / 2)]!;
  oreLight.position.set(mid[0] - half + 0.5, mid[1] + 2, mid[2] - half + 1.5);
  scene.add(oreLight);

  // Embers drifting up from the vein; bloom turns them into soft points of light.
  const emberCount = 40;
  const emberPos = new Float32Array(emberCount * 3);
  const emberSeed = Array.from({ length: emberCount }, (_, i) => {
    const o = world.ore[(i * 7) % world.ore.length]!;
    return { x: o[0] - half + Math.random(), y: o[1] + 1, z: o[2] - half + Math.random(), s: 0.15 + Math.random() * 0.35, p: Math.random() * 10 };
  });
  const embersGeo = new BufferGeometry();
  embersGeo.setAttribute('position', new BufferAttribute(emberPos, 3));
  const embers = new Points(embersGeo, new PointsMaterial({ color: 0xff7a55, size: 0.09, transparent: true, opacity: 0.9, blending: AdditiveBlending, depthWrite: false }));
  scene.add(embers);

  // Stars for dusk and night: fixed points on a far shell, faded in by the mood.
  const starPos = new Float32Array(500 * 3);
  for (let i = 0; i < 500; i++) {
    const u = Math.random() * Math.PI * 2;
    const v = Math.random() * 0.9 + 0.1;
    starPos.set([Math.cos(u) * Math.sqrt(1 - v * v) * 150, v * 150, Math.sin(u) * Math.sqrt(1 - v * v) * 150], i * 3);
  }
  const starGeo = new BufferGeometry();
  starGeo.setAttribute('position', new BufferAttribute(starPos, 3));
  const starMat = new PointsMaterial({ color: 0xdfe6ff, size: 1.4, sizeAttenuation: false, transparent: true, opacity: 0, depthWrite: false, fog: false });
  scene.add(new Points(starGeo, starMat));

  const hemi = new HemisphereLight(0xffffff, 0x222222, 1);
  scene.add(hemi);
  const key = new DirectionalLight(0xffffff, 2);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.camera.left = -14;
  key.shadow.camera.right = 14;
  key.shadow.camera.top = 14;
  key.shadow.camera.bottom = -14;
  key.shadow.camera.near = 1;
  key.shadow.camera.far = 80;
  key.shadow.bias = -0.0006;
  key.shadow.normalBias = 0.02;
  key.shadow.radius = 4;
  scene.add(key, key.target);
  // A cool rim from behind separates the silhouette from the sky.
  const rim = new DirectionalLight(0x9fb8ff, 0.9);
  rim.position.set(-20, 18, -30);
  scene.add(rim);

  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(scene, camera));
  const bloom = new UnrealBloomPass(new Vector2(256, 256), 0.55, 0.6, 0.82);
  composer.addPass(bloom);
  composer.addPass(new OutputPass());

  let view = options.frame;
  let paused = false;
  let refinedTarget = 1;
  const look = { x: 0, y: 0, tx: 0, ty: 0 };

  function applyMood(mood: Mood) {
    const m = MOODS[mood];
    skyMat.uniforms.top!.value.set(m.skyTop);
    skyMat.uniforms.horizon!.value.set(m.horizon);
    skyMat.uniforms.glow!.value.set(m.sunGlow);
    skyMat.uniforms.sunDir!.value.set(m.sunScreen[0], m.sunScreen[1], 0);
    (scene.fog as Fog).color.set(m.horizon);
    hemi.color.set(m.hemiSky);
    hemi.groundColor.set(m.hemiGround);
    hemi.intensity = m.hemi;
    key.color.set(m.key);
    key.intensity = m.keyIntensity;
    key.position.set(m.sun[0] * 40, m.sun[1] * 40, m.sun[2] * 40);
    glowGain.value = m.oreGlow;
    oreLight.intensity = m.oreLight;
    renderer.toneMappingExposure = m.exposure;
    bloom.strength = m.bloom;
    starMat.opacity = m.stars;
  }
  applyMood(options.mood);

  function frame(t: number) {
    // Slow orbit drift plus pointer parallax; both stay within a few degrees.
    const drift = options.still ? 0 : Math.sin(t * 0.07) * 0.06;
    look.x += (look.tx - look.x) * 0.05;
    look.y += (look.ty - look.y) * 0.05;
    const az = (options.orbit ?? 0.7) + drift + look.x * 0.05;
    const el = 0.27 + look.y * 0.03;
    const dist = 48;
    camera.position.set(Math.sin(az) * Math.cos(el) * dist, Math.sin(el) * dist + 2, Math.cos(az) * Math.cos(el) * dist);
    camera.lookAt(0, 1, 0);

    refine.value += (refinedTarget * world.ore.length - refine.value) * (options.still ? 1 : 0.12);
    if (!options.still) {
      for (let i = 0; i < emberCount; i++) {
        const e = emberSeed[i]!;
        const k = (t * e.s + e.p) % 4;
        emberPos[i * 3] = e.x + Math.sin(t * 0.6 + e.p) * 0.25;
        emberPos[i * 3 + 1] = e.y + k;
        emberPos[i * 3 + 2] = e.z + Math.cos(t * 0.5 + e.p) * 0.25;
      }
      embersGeo.attributes.position!.needsUpdate = true;
      (embers.material as PointsMaterial).opacity = refinedTarget > 0 ? 0.85 : 0;
    } else {
      embers.visible = false;
    }
    player.update(options.still ? 0 : t, look);
    for (const rock of rocks) {
      rock.mesh.position.y = rock.y + (options.still ? 0 : Math.sin(t * 0.35 + rock.phase) * 0.35);
      rock.mesh.rotation.y = options.still ? rock.phase : rock.phase + t * 0.02;
    }
    composer.render();
  }

  function resize() {
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    if (w === 0 || h === 0) return;
    renderer.setSize(w, h, false);
    skyMat.uniforms.resolution!.value.set(w * renderer.getPixelRatio(), h * renderer.getPixelRatio());
    composer.setSize(w, h);
    camera.aspect = w / h;
    // Shift the subject inside the frame without moving the camera, so perspective stays stable.
    camera.setViewOffset(w, h, -view.x * w, view.y * h, w, h);
    camera.zoom = view.zoom;
    camera.updateProjectionMatrix();
    if (options.still || paused) frame(0);
  }

  // Idle at 30 fps: smooth enough for drift, half the GPU time of a 60 Hz loop.
  let raf = 0;
  let last = 0;
  const start = performance.now();
  // If the display can't keep a steady cadence, the GPU can't afford the drift: settle on a still.
  let probes = 0;
  let probeStart = 0;
  function loop(now: number) {
    raf = requestAnimationFrame(loop);
    if (probes === 0) probeStart = now;
    if (++probes === 60 && (now - probeStart) / 59 > 40) {
      options.still = true;
      cancelAnimationFrame(raf);
      frame(0);
      return;
    }
    if (now - last < 1000 / 30) return;
    last = now;
    frame((now - start) / 1000);
  }

  const onVisibility = () => setPaused(paused);
  function setPaused(next: boolean) {
    paused = next;
    cancelAnimationFrame(raf);
    if (!paused && !options.still && document.visibilityState === 'visible') raf = requestAnimationFrame(loop);
  }
  document.addEventListener('visibilitychange', onVisibility);

  resize();
  frame(0);
  setPaused(false);

  return {
    setRefined(fraction) {
      refinedTarget = Math.max(0, Math.min(1, fraction));
      if (options.still || paused) frame(0);
    },
    setMood(mood) {
      applyMood(mood);
      if (options.still || paused) frame(0);
    },
    setFrame(next) {
      view = next;
      resize();
    },
    setPaused,
    pointer(x, y) {
      look.tx = x;
      look.ty = y;
    },
    resize,
    dispose() {
      cancelAnimationFrame(raf);
      document.removeEventListener('visibilitychange', onVisibility);
      composer.dispose();
      renderer.dispose();
      pmrem.dispose();
    },
  };
}
