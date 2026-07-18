import * as THREE from "three";

import {
  libraryShelves,
  volumesForShelf,
  type LibraryVolume,
  type VisualShelf,
} from "./projection";
import {
  SCENE_HEIGHT,
  SCENE_WIDTH,
  SHELF_MOTION_BOUNDS,
  advanceShelfMotion,
  cabinetScaleForCount,
  edgeShelfOrbitTarget,
  initialShelfMotion,
  majorEffectDelay,
  minorEffectDelay,
  orbitAroundGroundAxis,
  sealFrame,
  sealSymbol,
  shelfRoamingCells,
  spatialNeighbor,
  stellarDialTarget,
  topLeftShelf,
  type PixelSymbol,
  type ProjectedShelf,
  type ShelfRoamingCell,
  type ShelfMotion,
  type SpatialDirection,
} from "./scene-model";
import type { LibraryProjection } from "./types";

const COLORS = {
  void: 0x020203,
  ink: 0x08080a,
  charcoal: 0x17161a,
  darkMetal: 0x29262d,
  paper: 0xe8e3d8,
  bright: 0xfffcf2,
  dim: 0x817b83,
  purple: 0xa968d2,
  purpleBright: 0xd8a0ff,
  purpleDark: 0x371844,
  brass: 0x8f795d,
} as const;
const PURPLE_DARK_COLOR = new THREE.Color(COLORS.purpleDark);
const PURPLE_BRIGHT_COLOR = new THREE.Color(COLORS.purpleBright);

const SHELF_WIDTH = 2.75;
const SHELF_HEIGHT = 3.85;
const SEAL_HOUSING_SCALE = 0.9;
const FOCUS_POSITION = new THREE.Vector3(0, -0.04, 0.25);
const GROUND_ORBIT_CENTER = new THREE.Vector3(0, -2.38, -1.6);
const DRAG_LIMIT_X = SHELF_MOTION_BOUNDS.maximumX;
const DRAG_LIMIT_Y = SHELF_MOTION_BOUNDS.maximumY;

const VISUAL_CONTRACT = {
  camera: {
    fov: 43,
    near: 0.1,
    far: 80,
    position: new THREE.Vector3(0, 0.42, 9),
  },
  focus: {
    scale: 1,
    inactiveCenterClearance: 3.2,
    inactiveDepthOffset: 0.8,
  },
  pixel: {
    seed: 0x41a57a,
    debugParameter: "libraryDebug",
  },
  density: {
    cabinetCount: 7,
  },
  effects: {
    groundDuration: 6.4,
    starDuration: 3.2,
    mistDuration: 7.5,
  },
} as const;

export interface LibrarySceneCallbacks {
  onAimShelf(id: string): void;
  onSelectShelf(id: string): void;
  onSelectVolume(volume: LibraryVolume): void;
}

export interface SceneHandle {
  aimedShelfId(): string | null;
  focusedShelfId(): string | null;
  aimShelf(id: string): boolean;
  moveAim(direction: SpatialDirection): string | null;
  activateShelf(id: string): void;
  deactivateShelf(): void;
  openVolume(volumeId: string | null): void;
  select(bookId: string | null): void;
  setReducedMotion(value: boolean): void;
  destroy(): void;
}

interface VolumeVisual {
  volume: LibraryVolume;
  mesh: THREE.Mesh<THREE.BoxGeometry, THREE.MeshStandardMaterial>;
  labelMaterial: THREE.MeshBasicMaterial;
}

interface ShelfMagicStone {
  mesh: THREE.Mesh<THREE.OctahedronGeometry, THREE.MeshStandardMaterial>;
  material: THREE.MeshStandardMaterial;
  baseScale: THREE.Vector3;
  phase: number;
  role: "crown" | "frame" | "pendant";
}

interface CabinetMaterials {
  wood: THREE.MeshStandardMaterial;
  trim: THREE.MeshStandardMaterial;
  dark: THREE.MeshStandardMaterial;
}

interface CabinetVisual {
  stones: ShelfMagicStone[];
  materials: CabinetMaterials;
}

interface SealEnergyStream {
  line: THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial>;
  material: THREE.LineBasicMaterial;
  pointCount: number;
  phase: number;
}

interface SealVisual {
  housing: THREE.Group;
  lockLeaves: [THREE.Group, THREE.Group];
  root: THREE.Group;
  pool: THREE.Mesh<THREE.CircleGeometry, THREE.MeshBasicMaterial>;
  poolMaterial: THREE.MeshBasicMaterial;
  sigil: THREE.Group;
  sigilMaterials: THREE.Material[];
  glow: THREE.Sprite;
  glowMaterial: THREE.SpriteMaterial;
  unlock: THREE.Group;
  unlockMaterials: THREE.Material[];
  particles: THREE.Points<THREE.BufferGeometry, THREE.PointsMaterial>;
  particleOrigins: Float32Array;
  particleVelocities: Float32Array;
  particleTargets: Float32Array;
  particlePhases: Float32Array;
  energyStreams: SealEnergyStream[];
  stones: ShelfMagicStone[];
  lastProgress: number;
}

interface StellarDialLayer {
  line: THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial>;
  pointCount: number;
  start: number;
  duration: number;
  spin: number;
}

interface StellarDial {
  root: THREE.Group;
  materials: THREE.Material[];
  pointers: THREE.Group[];
  spirals: StellarDialLayer[];
}

interface ShelfActor {
  shelf: VisualShelf;
  sourceShelfId: string;
  cell: ShelfRoamingCell;
  layoutScale: number;
  root: THREE.Group;
  model: THREE.Group;
  motion: ShelfMotion;
  homePosition: THREE.Vector3;
  projected: ProjectedShelf;
  phase: number;
  aimGlow: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>;
  aimGlowMaterial: THREE.MeshBasicMaterial;
  aimLight: THREE.PointLight;
  aimProgress: number;
  focusLight: THREE.PointLight;
  cabinetMaterials: CabinetMaterials;
  dial: StellarDial;
  seal: SealVisual;
  volumes: VolumeVisual[];
  contentMaterials: THREE.Material[];
  focusProgress: number;
  focusTarget: 0 | 1;
  sealProgress: number;
  dialProgress: number;
  dragging: boolean;
  dragVelocityX: number;
  dragVelocityY: number;
  orbitYaw: number;
}

interface ShelfInstance {
  shelf: VisualShelf;
  sourceShelfId: string;
}

interface FogSprite {
  sprite: THREE.Sprite;
  baseX: number;
  baseY: number;
  baseOpacity: number;
  phase: number;
}

interface AmbientState {
  nextMinorAt: number;
  nextMajorAt: number;
  groundPulseStart: number;
  starFlashStart: number;
  mistSurgeStart: number;
}

interface GroundSigil {
  root: THREE.Group;
  base: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>;
  echoes: Array<THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>>;
}

interface CometVisual {
  line: THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial>;
  head: THREE.Sprite;
  headMaterial: THREE.SpriteMaterial;
  start: number;
  duration: number;
  from: THREE.Vector3;
  to: THREE.Vector3;
  bend: number;
}

interface LightningVisual {
  line: THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial>;
  start: number;
  duration: number;
}

interface FogSilhouette {
  sprite: THREE.Sprite;
  material: THREE.SpriteMaterial;
  baseX: number;
  baseY: number;
  start: number;
  duration: number;
  phase: number;
}

interface NebulaVisual {
  sprite: THREE.Sprite;
  material: THREE.SpriteMaterial;
  start: number;
  duration: number;
  phase: number;
  base: THREE.Vector3;
  baseScale: THREE.Vector2;
}

interface DistantAperture {
  sprite: THREE.Sprite;
  material: THREE.SpriteMaterial;
  start: number;
  duration: number;
  phase: number;
  base: THREE.Vector3;
}

interface BackgroundEffects {
  comets: CometVisual[];
  lightning: LightningVisual[];
  silhouettes: FogSilhouette[];
  nebulae: NebulaVisual[];
  apertures: DistantAperture[];
}

interface RisingPixels {
  points: THREE.Points<THREE.BufferGeometry, THREE.PointsMaterial>;
  speeds: Float32Array;
}

interface DragState {
  actor: ShelfActor;
  canDrag: boolean;
  pointerId: number;
  startX: number;
  startY: number;
  moved: boolean;
  plane: THREE.Plane;
  offset: THREE.Vector3;
  lastWorld: THREE.Vector3;
  lastTime: number;
}

type SceneDebugMode = "final" | "raw" | "monochrome" | "contours";

interface PixelPipeline {
  render(
    world: THREE.Scene,
    camera: THREE.PerspectiveCamera,
    overlay: THREE.Scene,
    overlayCamera: THREE.OrthographicCamera,
  ): void;
  dispose(): void;
}

export async function mountLibraryScene(
  host: HTMLElement,
  projection: LibraryProjection,
  initialAimedShelfId: string | null,
  initialFocusedShelfId: string | null,
  activeVolumeId: string | null,
  reducedMotion: boolean,
  callbacks: LibrarySceneCallbacks,
): Promise<SceneHandle> {
  await document.fonts.load('72px "Departure Mono"');
  const renderer = new THREE.WebGLRenderer({
    antialias: false,
    alpha: false,
    powerPreference: "high-performance",
    precision: "highp",
  });
  renderer.setPixelRatio(1);
  renderer.setSize(SCENE_WIDTH, SCENE_HEIGHT, false);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.domElement.className = "world-canvas";
  renderer.domElement.setAttribute("aria-hidden", "true");
  renderer.domElement.style.touchAction = "none";
  host.replaceChildren(renderer.domElement);

  const world = new THREE.Scene();
  world.background = new THREE.Color(COLORS.void);
  const camera = new THREE.PerspectiveCamera(
    VISUAL_CONTRACT.camera.fov,
    SCENE_WIDTH / SCENE_HEIGHT,
    VISUAL_CONTRACT.camera.near,
    VISUAL_CONTRACT.camera.far,
  );
  camera.position.copy(VISUAL_CONTRACT.camera.position);
  camera.lookAt(0, 0, -1);

  const overlay = new THREE.Scene();
  const overlayCamera = new THREE.OrthographicCamera(
    -SCENE_WIDTH / 2,
    SCENE_WIDTH / 2,
    SCENE_HEIGHT / 2,
    -SCENE_HEIGHT / 2,
    0,
    10,
  );
  overlayCamera.position.z = 5;

  const random = deterministicRandom(VISUAL_CONTRACT.pixel.seed);
  const debugMode = sceneDebugMode();
  const pixelPipeline = createPixelPipeline(renderer, debugMode);
  renderer.domElement.dataset.visualMode = debugMode;
  addLighting(world);
  const stars = createStarField(world, random);
  const groundCircle = createGroundCircle(world);
  const backgroundEffects = createBackgroundEffects(world, overlay, random);
  const fogSprites = createCornerFog(overlay);
  const worldMotes = createWorldMotes(world, random);

  const shelfLayer = new THREE.Group();
  world.add(shelfLayer);
  const shelfPickables: THREE.Object3D[] = [];
  const volumePickables: THREE.Object3D[] = [];
  const shelves = libraryShelves(projection);
  const shelfInstances = createShelfInstances(
    shelves,
    Math.max(VISUAL_CONTRACT.density.cabinetCount, shelves.length),
  );
  const cells = shelfRoamingCells(shelfInstances.length);
  const layoutScale = cabinetScaleForCount(shelfInstances.length);
  const actors = shelfInstances.map((instance, index) =>
    createShelfActor(
      shelfLayer,
      instance.shelf,
      instance.sourceShelfId,
      cells[index]!,
      layoutScale,
      index,
      random,
      shelfPickables,
      volumePickables,
    ),
  );
  const actorsById = new Map(actors.map((actor) => [actor.shelf.id, actor]));
  const primaryActorsBySource = new Map<string, ShelfActor>();
  for (const actor of actors) {
    if (!primaryActorsBySource.has(actor.sourceShelfId)) {
      primaryActorsBySource.set(actor.sourceShelfId, actor);
    }
  }
  for (const actor of actors) {
    actor.homePosition.copy(motionToWorld(actor.motion));
    actor.root.position.copy(actor.homePosition);
    actor.projected = projectWorldPosition(actor.shelf.id, actor.root.position);
  }

  let aimedId = (initialAimedShelfId
    ? primaryActorsBySource.get(initialAimedShelfId)?.shelf.id
    : undefined) ?? topLeftShelf(actors.map((actor) => actor.projected));
  let focusedId = initialFocusedShelfId
    ? primaryActorsBySource.get(initialFocusedShelfId)?.shelf.id ?? null
    : null;
  if (focusedId) {
    aimedId = focusedId;
  }
  const sourceIdForVisual = (visualId: string | null): string | null =>
    visualId ? actorsById.get(visualId)?.sourceShelfId ?? null : null;
  const actorForSource = (sourceShelfId: string): ShelfActor | undefined => {
    const aimedActor = aimedId ? actorsById.get(aimedId) : undefined;
    if (aimedActor?.sourceShelfId === sourceShelfId) {
      return aimedActor;
    }
    const focusedActor = focusedId ? actorsById.get(focusedId) : undefined;
    if (focusedActor?.sourceShelfId === sourceShelfId) {
      return focusedActor;
    }
    return primaryActorsBySource.get(sourceShelfId);
  };
  let openedVolumeId = activeVolumeId;
  let selectedBookId: string | null = null;
  let motionReduced = reducedMotion;
  let elapsed = 0;
  let previousTime = performance.now();
  let animationFrame: number | null = null;
  let destroyed = false;
  let dragState: DragState | null = null;
  const raycaster = new THREE.Raycaster();
  const pointer = new THREE.Vector2();
  const scratchWorld = new THREE.Vector3();
  const scratchDirection = new THREE.Vector3();
  const ambient: AmbientState = {
    nextMinorAt: 9,
    nextMajorAt: 28,
    groundPulseStart: Number.NEGATIVE_INFINITY,
    starFlashStart: Number.NEGATIVE_INFINITY,
    mistSurgeStart: Number.NEGATIVE_INFINITY,
  };

  for (const actor of actors) {
    actor.focusTarget = actor.shelf.id === focusedId ? 1 : 0;
    if (actor.focusTarget === 1) {
      actor.focusProgress = 1;
      actor.sealProgress = 1;
      actor.dialProgress = 1;
    }
  }

  const render = (): void => {
    pixelPipeline.render(world, camera, overlay, overlayCamera);
  };

  const update = (seconds: number): void => {
    if (!motionReduced) {
      elapsed += seconds;
      updateFreeMotion(actors, seconds);
    }
    updateActors(
      actors,
      camera,
      aimedId,
      focusedId,
      openedVolumeId,
      selectedBookId,
      elapsed,
      seconds,
      motionReduced,
    );
    updateAmbient(
      stars,
      groundCircle,
      fogSprites,
      worldMotes,
      backgroundEffects,
      ambient,
      elapsed,
      seconds,
      motionReduced,
      random,
    );
    render();
  };

  const animate = (time: number): void => {
    animationFrame = null;
    if (destroyed || document.hidden || motionReduced) {
      return;
    }
    const seconds = Math.min(Math.max((time - previousTime) / 1000, 0), 0.05);
    previousTime = time;
    update(seconds);
    animationFrame = requestAnimationFrame(animate);
  };

  const startAnimation = (): void => {
    if (animationFrame !== null || destroyed || document.hidden || motionReduced) {
      return;
    }
    previousTime = performance.now();
    animationFrame = requestAnimationFrame(animate);
  };

  const stopAnimation = (): void => {
    if (animationFrame !== null) {
      cancelAnimationFrame(animationFrame);
      animationFrame = null;
    }
  };

  function aimShelf(id: string): boolean {
    if (focusedId) {
      return false;
    }
    const next = actorForSource(id);
    if (!next) {
      return false;
    }
    aimedId = next.shelf.id;
    if (motionReduced) {
      update(0);
    }
    return true;
  }

  function moveAim(direction: SpatialDirection): string | null {
    if (focusedId || !aimedId) {
      return sourceIdForVisual(aimedId);
    }
    const next = spatialNeighbor(
      aimedId,
      actors.map((actor) => actor.projected),
      direction,
    );
    if (next) {
      aimedId = next;
      const nextActor = actorsById.get(next);
      if (nextActor) {
        callbacks.onAimShelf(nextActor.sourceShelfId);
      }
      if (motionReduced) {
        update(0);
      }
    }
    return sourceIdForVisual(aimedId);
  }

  function activateShelf(id: string): void {
    const next = actorForSource(id);
    if (!next) {
      return;
    }
    aimedId = next.shelf.id;
    focusedId = next.shelf.id;
    openedVolumeId = null;
    selectedBookId = null;
    next.homePosition.copy(motionToWorld(next.motion));
    for (const actor of actors) {
      actor.focusTarget = actor.shelf.id === next.shelf.id ? 1 : 0;
    }
    if (motionReduced) {
      update(0);
    }
  }

  function deactivateShelf(): void {
    if (!focusedId) {
      return;
    }
    for (const actor of actors) {
      actor.focusTarget = 0;
    }
    focusedId = null;
    openedVolumeId = null;
    selectedBookId = null;
    if (motionReduced) {
      update(0);
    }
  }

  const setPointer = (event: PointerEvent): void => {
    const bounds = renderer.domElement.getBoundingClientRect();
    pointer.x = ((event.clientX - bounds.left) / bounds.width) * 2 - 1;
    pointer.y = -((event.clientY - bounds.top) / bounds.height) * 2 + 1;
    raycaster.setFromCamera(pointer, camera);
  };

  const shelfFromObject = (object: THREE.Object3D | undefined): ShelfActor | undefined => {
    const id = object?.userData.shelfId;
    return typeof id === "string" ? actorsById.get(id) : undefined;
  };

  const pointerDown = (event: PointerEvent): void => {
    if (event.button !== 0 || dragState) {
      return;
    }
    setPointer(event);
    const hit = raycaster.intersectObjects(shelfPickables, false)[0];
    const actor = shelfFromObject(hit?.object);
    if (!actor) {
      return;
    }
    aimedId = actor.shelf.id;
    callbacks.onAimShelf(actor.sourceShelfId);
    camera.getWorldDirection(scratchDirection);
    const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(
      scratchDirection,
      actor.root.position,
    );
    const intersection = raycaster.ray.intersectPlane(plane, scratchWorld);
    if (!intersection) {
      return;
    }
    const canDrag = actor.focusTarget === 0;
    actor.dragging = canDrag;
    if (canDrag && Math.abs(actor.orbitYaw) > 0.0001) {
      actor.motion = clampMotionToCell(worldToMotion(actor.root.position), actor.cell);
      actor.homePosition.copy(motionToWorld(actor.motion));
      actor.orbitYaw = 0;
    }
    dragState = {
      actor,
      canDrag,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      moved: false,
      plane,
      offset: actor.root.position.clone().sub(intersection),
      lastWorld: actor.root.position.clone(),
      lastTime: performance.now(),
    };
    renderer.domElement.setPointerCapture(event.pointerId);
    renderer.domElement.style.cursor = "grabbing";
    if (motionReduced) {
      update(0);
    }
  };

  const pointerMove = (event: PointerEvent): void => {
    if (!dragState || dragState.pointerId !== event.pointerId) {
      return;
    }
    if (!dragState.canDrag) {
      return;
    }
    setPointer(event);
    const intersection = raycaster.ray.intersectPlane(dragState.plane, scratchWorld);
    if (!intersection) {
      return;
    }
    const worldPosition = intersection.clone().add(dragState.offset);
    const motion = worldToMotion(worldPosition);
    dragState.actor.motion.x = clamp(
      motion.x,
      dragState.actor.cell.minimumX,
      dragState.actor.cell.maximumX,
    );
    dragState.actor.motion.y = clamp(
      motion.y,
      dragState.actor.cell.minimumY,
      dragState.actor.cell.maximumY,
    );
    dragState.actor.homePosition.copy(motionToWorld(dragState.actor.motion));
    dragState.actor.root.position.copy(dragState.actor.homePosition);
    const now = performance.now();
    const seconds = Math.max((now - dragState.lastTime) / 1000, 0.001);
    dragState.actor.dragVelocityX = (worldPosition.x - dragState.lastWorld.x) / seconds;
    dragState.actor.dragVelocityY = (worldPosition.y - dragState.lastWorld.y) / seconds;
    dragState.lastWorld.copy(worldPosition);
    dragState.lastTime = now;
    dragState.moved ||= Math.hypot(
      event.clientX - dragState.startX,
      event.clientY - dragState.startY,
    ) > 5;
    if (motionReduced) {
      update(0);
    }
  };

  const finishPointer = (event: PointerEvent): void => {
    if (!dragState || dragState.pointerId !== event.pointerId) {
      return;
    }
    const finished = dragState;
    dragState = null;
    finished.actor.dragging = false;
    renderer.domElement.style.cursor = "default";
    if (renderer.domElement.hasPointerCapture(event.pointerId)) {
      renderer.domElement.releasePointerCapture(event.pointerId);
    }
    if (finished.moved) {
      const inertia = 0.0007;
      finished.actor.motion.velocityX = clamp(
        finished.actor.motion.velocityX + finished.actor.dragVelocityX * inertia,
        -0.04,
        0.04,
      );
      finished.actor.motion.velocityY = clamp(
        finished.actor.motion.velocityY + finished.actor.dragVelocityY * inertia,
        -0.035,
        0.035,
      );
      if (motionReduced) {
        update(0);
      }
      return;
    }
    setPointer(event);
    const volumeHit = raycaster.intersectObjects(volumePickables, false)[0];
    const volume = volumeHit?.object.userData.volume as LibraryVolume | undefined;
    const actor = shelfFromObject(volumeHit?.object);
    if (
      volume &&
      actor?.shelf.id === focusedId &&
      actor.sealProgress > 0.92
    ) {
      callbacks.onSelectVolume(volume);
      return;
    }
    callbacks.onSelectShelf(finished.actor.sourceShelfId);
  };

  const pointerCancel = (event: PointerEvent): void => {
    if (!dragState || dragState.pointerId !== event.pointerId) {
      return;
    }
    dragState.actor.dragging = false;
    dragState = null;
    renderer.domElement.style.cursor = "default";
  };

  const visibilityChanged = (): void => {
    if (document.hidden || motionReduced) {
      stopAnimation();
    } else {
      startAnimation();
    }
  };

  renderer.domElement.addEventListener("pointerdown", pointerDown);
  renderer.domElement.addEventListener("pointermove", pointerMove);
  renderer.domElement.addEventListener("pointerup", finishPointer);
  renderer.domElement.addEventListener("pointercancel", pointerCancel);
  document.addEventListener("visibilitychange", visibilityChanged);
  update(0);
  startAnimation();

  return {
    aimedShelfId: () => sourceIdForVisual(aimedId),
    focusedShelfId: () => sourceIdForVisual(focusedId),
    aimShelf,
    moveAim,
    activateShelf,
    deactivateShelf,
    openVolume(volumeId: string | null): void {
      openedVolumeId = volumeId;
      if (motionReduced) {
        update(0);
      }
    },
    select(bookId: string | null): void {
      selectedBookId = bookId;
      if (motionReduced) {
        update(0);
      }
    },
    setReducedMotion(value: boolean): void {
      if (motionReduced === value) {
        return;
      }
      motionReduced = value;
      if (motionReduced) {
        stopAnimation();
        update(0);
      } else {
        startAnimation();
      }
    },
    destroy(): void {
      destroyed = true;
      stopAnimation();
      document.removeEventListener("visibilitychange", visibilityChanged);
      renderer.domElement.removeEventListener("pointerdown", pointerDown);
      renderer.domElement.removeEventListener("pointermove", pointerMove);
      renderer.domElement.removeEventListener("pointerup", finishPointer);
      renderer.domElement.removeEventListener("pointercancel", pointerCancel);
      disposeScene(world);
      disposeScene(overlay);
      pixelPipeline.dispose();
      renderer.dispose();
      renderer.domElement.remove();
    },
  };
}

function createPixelPipeline(
  renderer: THREE.WebGLRenderer,
  debugMode: SceneDebugMode,
): PixelPipeline {
  const target = new THREE.WebGLRenderTarget(SCENE_WIDTH, SCENE_HEIGHT, {
    minFilter: THREE.NearestFilter,
    magFilter: THREE.NearestFilter,
    format: THREE.RGBAFormat,
    type: THREE.UnsignedByteType,
    depthBuffer: true,
    stencilBuffer: false,
  });
  target.texture.generateMipmaps = false;

  const postScene = new THREE.Scene();
  const postCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const postGeometry = new THREE.PlaneGeometry(2, 2);
  const postMaterial = new THREE.ShaderMaterial({
    uniforms: {
      sceneTexture: { value: target.texture },
      voidColor: { value: new THREE.Color(COLORS.void) },
      paperColor: { value: new THREE.Color(COLORS.paper) },
      accentColor: { value: new THREE.Color(COLORS.purpleBright) },
      texelSize: { value: new THREE.Vector2(1 / SCENE_WIDTH, 1 / SCENE_HEIGHT) },
      monochrome: { value: debugMode === "monochrome" ? 1 : 0 },
      contours: { value: debugMode === "contours" ? 1 : 0 },
    },
    vertexShader: `
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = vec4(position.xy, 0.0, 1.0);
      }
    `,
    fragmentShader: `
      uniform sampler2D sceneTexture;
      uniform vec3 voidColor;
      uniform vec3 paperColor;
      uniform vec3 accentColor;
      uniform vec2 texelSize;
      uniform float monochrome;
      uniform float contours;
      varying vec2 vUv;

      float hash21(vec2 value) {
        vec3 p = fract(vec3(value.xyx) * 0.1031);
        p += dot(p, p.yzx + 33.33);
        return fract((p.x + p.y) * p.z);
      }

      float clusteredDot(vec2 position) {
        vec2 cell = floor(position / 10.0);
        vec2 pixel = mod(floor(position), 10.0) - vec2(4.5);
        float turn = floor(hash21(cell + 7.13) * 4.0);
        if (turn == 1.0) {
          pixel = vec2(-pixel.y, pixel.x);
        } else if (turn == 2.0) {
          pixel = -pixel;
        } else if (turn == 3.0) {
          pixel = vec2(pixel.y, -pixel.x);
        }
        vec2 offset = vec2(
          hash21(cell + 17.2) - 0.5,
          hash21(cell + 41.7) - 0.5
        ) * 2.1;
        float radial = (length(pixel - offset) - 0.24) / 6.1;
        return clamp(radial + (hash21(cell * 1.73) - 0.5) * 0.68, 0.018, 0.985);
      }

      void main() {
        vec2 pixelUv =
          (floor(vUv / texelSize / 2.0) * 2.0 + vec2(1.0)) * texelSize;
        vec3 source = texture2D(sceneTexture, pixelUv).rgb;
        float luminance = dot(source, vec3(0.2126, 0.7152, 0.0722));
        float leftLuminance = dot(
          texture2D(sceneTexture, pixelUv - vec2(texelSize.x * 2.0, 0.0)).rgb,
          vec3(0.2126, 0.7152, 0.0722)
        );
        float rightLuminance = dot(
          texture2D(sceneTexture, pixelUv + vec2(texelSize.x * 2.0, 0.0)).rgb,
          vec3(0.2126, 0.7152, 0.0722)
        );
        float lowerLuminance = dot(
          texture2D(sceneTexture, pixelUv - vec2(0.0, texelSize.y * 2.0)).rgb,
          vec3(0.2126, 0.7152, 0.0722)
        );
        float upperLuminance = dot(
          texture2D(sceneTexture, pixelUv + vec2(0.0, texelSize.y * 2.0)).rgb,
          vec3(0.2126, 0.7152, 0.0722)
        );
        float neighborMaximum = max(max(leftLuminance, rightLuminance), max(lowerLuminance, upperLuminance));
        float neighborMinimum = min(min(leftLuminance, rightLuminance), min(lowerLuminance, upperLuminance));
        float contourCoverage = smoothstep(0.028, 0.16, neighborMaximum - neighborMinimum);
        float threshold = clusteredDot(gl_FragCoord.xy);
        float tonalCoverage = smoothstep(
          0.018,
          0.88,
          pow(clamp(luminance * 1.18, 0.0, 1.0), 0.78)
        );
        tonalCoverage = clamp(tonalCoverage + contourCoverage * 0.22, 0.0, 1.0);
        float quantizedTone = floor(tonalCoverage * 6.0 + 0.5) / 6.0;
        float tonalBands = mix(tonalCoverage, quantizedTone, 0.48);
        float shadowCluster =
          (threshold - 0.5) * 0.065 *
          smoothstep(0.035, 0.2, tonalBands) *
          (1.0 - smoothstep(0.28, 0.72, tonalBands));
        float paperCoverage = clamp(tonalBands - shadowCluster, 0.0, 1.0);
        vec3 neutral = mix(voidColor, paperColor, paperCoverage);
        vec3 result = neutral;

        float purpleSignal =
          max(source.b - source.g * 0.92, 0.0) +
          max(source.r - source.g * 1.04, 0.0) * 0.5;
        float accentWeight = smoothstep(0.055, 0.28, purpleSignal) * (1.0 - monochrome);
        float accentCoverage = clamp(
          max(paperCoverage * 0.58, purpleSignal * 1.22 + luminance * 0.12),
          0.0,
          1.0
        );
        result = mix(result, mix(voidColor, accentColor, accentCoverage), accentWeight);
        if (contours > 0.5) {
          result = mix(voidColor, paperColor, contourCoverage);
        }
        gl_FragColor = vec4(result, 1.0);
      }
    `,
    depthTest: false,
    depthWrite: false,
    toneMapped: false,
  });
  postScene.add(new THREE.Mesh(postGeometry, postMaterial));

  const renderLayers = (
    world: THREE.Scene,
    camera: THREE.PerspectiveCamera,
    overlay: THREE.Scene,
    overlayCamera: THREE.OrthographicCamera,
  ): void => {
    renderer.autoClear = true;
    renderer.render(world, camera);
    renderer.autoClear = false;
    renderer.clearDepth();
    renderer.render(overlay, overlayCamera);
    renderer.autoClear = true;
  };

  return {
    render(world, camera, overlay, overlayCamera): void {
      if (debugMode === "raw") {
        renderer.setRenderTarget(null);
        renderLayers(world, camera, overlay, overlayCamera);
        return;
      }
      renderer.setRenderTarget(target);
      renderLayers(world, camera, overlay, overlayCamera);
      renderer.setRenderTarget(null);
      renderer.render(postScene, postCamera);
    },
    dispose(): void {
      target.dispose();
      postGeometry.dispose();
      postMaterial.dispose();
      postScene.clear();
    },
  };
}

function sceneDebugMode(): SceneDebugMode {
  const value = new URLSearchParams(window.location.search).get(
    VISUAL_CONTRACT.pixel.debugParameter,
  );
  return value === "raw" || value === "monochrome" || value === "contours"
    ? value
    : "final";
}

function createShelfInstances(shelves: VisualShelf[], count: number): ShelfInstance[] {
  if (shelves.length === 0) {
    return [];
  }
  return Array.from({ length: count }, (_, index) => {
    const source = shelves[index % shelves.length]!;
    const mirrorNumber = Math.floor(index / shelves.length) + 1;
    if (mirrorNumber === 1) {
      return { shelf: source, sourceShelfId: source.id };
    }
    return {
      shelf: {
        ...source,
        id: `mirror:${encodeURIComponent(source.id)}:${mirrorNumber}`,
        label: `${source.label} / MIRROR ${romanNumeral(mirrorNumber)}`,
        status: `${source.status} mirror`,
      },
      sourceShelfId: source.id,
    };
  });
}

function romanNumeral(value: number): string {
  const numerals: ReadonlyArray<readonly [number, string]> = [
    [10, "X"], [9, "IX"], [5, "V"], [4, "IV"], [1, "I"],
  ];
  let remaining = value;
  let output = "";
  for (const [unit, glyph] of numerals) {
    while (remaining >= unit) {
      output += glyph;
      remaining -= unit;
    }
  }
  return output;
}

function createShelfActor(
  parent: THREE.Group,
  shelf: VisualShelf,
  sourceShelfId: string,
  cell: ShelfRoamingCell,
  layoutScale: number,
  index: number,
  random: () => number,
  shelfPickables: THREE.Object3D[],
  volumePickables: THREE.Object3D[],
): ShelfActor {
  const root = new THREE.Group();
  root.name = `shelf:${shelf.id}`;
  const model = new THREE.Group();
  root.add(model);
  const motion = initialShelfMotion(shelf.id, index, cell);
  const homePosition = motionToWorld(motion);
  root.position.copy(homePosition);
  const contentMaterials: THREE.Material[] = [];
  const cabinet = drawShelfCabinet(model, shelf, index, shelfPickables);
  const volumes = drawShelfBooks(
    model,
    shelf,
    index,
    contentMaterials,
    shelfPickables,
    volumePickables,
  );
  for (const volume of volumes) {
    volume.volume.id = volume.volume.id.replace(
      `${shelf.id}/`,
      `${sourceShelfId}/`,
    );
    volume.volume.shelfId = sourceShelfId;
  }
  const aim = createAimGlow(model);
  const dial = createStellarDial(model);
  const semanticLabel = [
    shelf.label,
    ...shelf.categories.map((category) => category.note_type),
  ].join(" ");
  const seal = createSeal(
    model,
    sealSymbol(semanticLabel, shelf.kind === "global"),
    random,
    cabinet.stones,
  );
  const focusLight = new THREE.PointLight(COLORS.paper, 0, 6.8, 1.55);
  focusLight.position.set(0, 0.3, 2.45);
  model.add(focusLight);
  parent.add(root);
  return {
    shelf,
    sourceShelfId,
    cell,
    layoutScale,
    root,
    model,
    motion,
    homePosition,
    projected: projectWorldPosition(shelf.id, root.position),
    phase: random() * Math.PI * 2,
    aimGlow: aim.sprite,
    aimGlowMaterial: aim.material,
    aimLight: aim.light,
    aimProgress: 0,
    focusLight,
    cabinetMaterials: cabinet.materials,
    dial,
    seal,
    volumes,
    contentMaterials,
    focusProgress: 0,
    focusTarget: 0,
    sealProgress: 0,
    dialProgress: 0,
    dragging: false,
    dragVelocityX: 0,
    dragVelocityY: 0,
    orbitYaw: 0,
  };
}

function drawShelfCabinet(
  parent: THREE.Group,
  shelf: VisualShelf,
  index: number,
  pickables: THREE.Object3D[],
): CabinetVisual {
  const magicStones: ShelfMagicStone[] = [];
  const wood = new THREE.MeshStandardMaterial({
    color: index % 2 === 0 ? 0x584650 : 0x624b59,
    emissive: 0x211923,
    emissiveIntensity: 0.95,
    roughness: 0.78,
    metalness: 0.12,
    flatShading: true,
  });
  const trim = new THREE.MeshStandardMaterial({
    color: 0xc2bac5,
    emissive: 0x3d3542,
    emissiveIntensity: 0.82,
    roughness: 0.58,
    metalness: 0.42,
    flatShading: true,
  });
  const dark = new THREE.MeshStandardMaterial({
    color: 0x242029,
    emissive: 0x100d13,
    emissiveIntensity: 0.82,
    roughness: 0.9,
    metalness: 0.05,
  });

  addCabinetBox(parent, shelf.id, [2.42, 3.34, 0.28], [0, -0.04, -0.19], dark, pickables);
  addCabinetBox(parent, shelf.id, [0.24, 3.5, 0.58], [-1.27, -0.03, 0], wood, pickables);
  addCabinetBox(parent, shelf.id, [0.24, 3.5, 0.58], [1.27, -0.03, 0], wood, pickables);
  addCabinetBox(parent, shelf.id, [0.13, 3.18, 0.68], [-1.08, -0.08, 0.12], trim, pickables);
  addCabinetBox(parent, shelf.id, [0.13, 3.18, 0.68], [1.08, -0.08, 0.12], trim, pickables);

  for (const [width, height, depth, y] of [
    [2.78, 0.18, 0.7, 1.72],
    [2.98, 0.15, 0.76, 1.89],
    [2.66, 0.13, 0.66, 2.04],
    [2.86, 0.18, 0.72, -1.79],
    [2.98, 0.13, 0.78, -1.94],
  ] as const) {
    addCabinetBox(parent, shelf.id, [width, height, depth], [0, y, 0.06], trim, pickables);
  }

  for (const y of [0.96, 0.06, -0.84, -1.62]) {
    addCabinetBox(parent, shelf.id, [2.35, 0.13, 0.63], [0, y, 0.08], trim, pickables);
    addCabinetBox(parent, shelf.id, [2.22, 0.06, 0.72], [0, y + 0.09, 0.15], wood, pickables);
  }

  for (const side of [-1, 1]) {
    for (const y of [-1.35, -0.48, 0.4, 1.27]) {
      const jewel = createMagicStone(0.075, side * 0.9 + y * 0.12);
      jewel.position.set(side * 1.28, y, 0.42);
      jewel.rotation.z = Math.PI / 4;
      jewel.userData.shelfId = shelf.id;
      parent.add(jewel);
      pickables.push(jewel);
      magicStones.push(magicStoneVisual(jewel, side * 1.7 + y, "frame"));
    }
    const finial = new THREE.Mesh(new THREE.ConeGeometry(0.13, 0.44, 4), trim);
    finial.position.set(side * 0.96, 2.28, 0.05);
    finial.rotation.y = Math.PI / 4;
    finial.userData.shelfId = shelf.id;
    parent.add(finial);
    pickables.push(finial);
  }

  for (const [crownIndex, x] of [-0.46, 0, 0.46].entries()) {
    const crown = createMagicStone(crownIndex === 1 ? 0.18 : 0.115, index + crownIndex * 0.8);
    crown.position.set(x, crownIndex === 1 ? 2.36 : 2.24, 0.16);
    crown.scale.y = crownIndex === 1 ? 1.25 : 1.08;
    crown.rotation.z = Math.PI / 4;
    crown.userData.shelfId = shelf.id;
    parent.add(crown);
    pickables.push(crown);
    magicStones.push(magicStoneVisual(crown, index + crownIndex * 0.8, "crown"));
  }

  const crownLines = lineSegments(
    [
      -1.22, 2.06, 0.25, -0.76, 2.24, 0.25,
      -0.76, 2.24, 0.25, -0.42, 2.12, 0.25,
      -0.42, 2.12, 0.25, 0, 2.42, 0.25,
      0, 2.42, 0.25, 0.42, 2.12, 0.25,
      0.42, 2.12, 0.25, 0.76, 2.24, 0.25,
      0.76, 2.24, 0.25, 1.22, 2.06, 0.25,
    ],
    COLORS.paper,
    0.74,
  );
  parent.add(crownLines);

  const titlePlate = addCabinetBox(
    parent,
    shelf.id,
    [2.14, 0.34, 0.14],
    [0, 1.7, 0.5],
    wood,
    pickables,
  );
  titlePlate.rotation.x = -0.04;
  const title = labelPlane(shelf.label, 2.08, 0.27, 68, COLORS.bright, 0x070708);
  title.position.set(0, 1.7, 0.585);
  title.userData.shelfId = shelf.id;
  parent.add(title);
  pickables.push(title);

  addBaroqueCabinetOrnaments(parent, shelf.id, wood, trim, pickables);

  for (const side of [-1, 1]) {
    const chainMaterial = new THREE.LineBasicMaterial({
      color: COLORS.dim,
      transparent: true,
      opacity: 0.72,
    });
    const chainGeometry = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(side * 1.02, -1.95, 0),
      new THREE.Vector3(side * 1.02, -2.36, -0.05),
    ]);
    parent.add(new THREE.Line(chainGeometry, chainMaterial));
    const crystal = createMagicStone(0.1, side * 2.3 + index);
    crystal.scale.y = 1.55;
    crystal.position.set(side * 1.02, -2.48, -0.05);
    parent.add(crystal);
    magicStones.push(magicStoneVisual(crystal, side * 2.3 + index, "pendant"));
  }
  return { stones: magicStones, materials: { wood, trim, dark } };
}

function createMagicStone(
  radius: number,
  phase: number,
): THREE.Mesh<THREE.OctahedronGeometry, THREE.MeshStandardMaterial> {
  const material = new THREE.MeshStandardMaterial({
    color: COLORS.purpleBright,
    emissive: COLORS.purpleDark,
    emissiveIntensity: 2.2,
    roughness: 0.28,
    metalness: 0.36,
    flatShading: true,
  });
  const stone = new THREE.Mesh(new THREE.OctahedronGeometry(radius, 0), material);
  stone.userData.energyPhase = phase;
  return stone;
}

function magicStoneVisual(
  mesh: THREE.Mesh<THREE.OctahedronGeometry, THREE.MeshStandardMaterial>,
  phase: number,
  role: ShelfMagicStone["role"],
): ShelfMagicStone {
  return {
    mesh,
    material: mesh.material,
    baseScale: mesh.scale.clone(),
    phase,
    role,
  };
}

function addBaroqueCabinetOrnaments(
  parent: THREE.Group,
  shelfId: string,
  wood: THREE.MeshStandardMaterial,
  trim: THREE.MeshStandardMaterial,
  pickables: THREE.Object3D[],
): void {
  for (const side of [-1, 1]) {
    const column = new THREE.Mesh(
      new THREE.CylinderGeometry(0.1, 0.145, 3.15, 8, 1, false),
      trim,
    );
    column.position.set(side * 1.13, -0.03, 0.43);
    column.userData.shelfId = shelfId;
    parent.add(column);
    pickables.push(column);
    for (const y of [-1.58, 1.5]) {
      addCabinetBox(
        parent,
        shelfId,
        [0.36, 0.22, 0.72],
        [side * 1.13, y, 0.18],
        trim,
        pickables,
      );
      addCabinetBox(
        parent,
        shelfId,
        [0.27, 0.12, 0.8],
        [side * 1.13, y + Math.sign(y) * 0.14, 0.2],
        wood,
        pickables,
      );
    }

    const upperScroll = baroqueCurve([
      new THREE.Vector3(side * 1.46, 2.02, 0.46),
      new THREE.Vector3(side * 1.3, 2.24, 0.49),
      new THREE.Vector3(side * 1.02, 2.32, 0.5),
      new THREE.Vector3(side * 0.82, 2.18, 0.51),
      new THREE.Vector3(side * 0.98, 2.08, 0.52),
      new THREE.Vector3(side * 0.68, 2.12, 0.53),
      new THREE.Vector3(side * 0.44, 2.32, 0.54),
      new THREE.Vector3(0, 2.48, 0.55),
    ], COLORS.paper, 0.78);
    upperScroll.userData.shelfId = shelfId;
    parent.add(upperScroll);

    const lowerScroll = baroqueCurve([
      new THREE.Vector3(side * 1.45, -1.93, 0.42),
      new THREE.Vector3(side * 1.27, -2.13, 0.44),
      new THREE.Vector3(side * 0.98, -2.2, 0.45),
      new THREE.Vector3(side * 0.77, -2.08, 0.46),
      new THREE.Vector3(side * 0.91, -1.98, 0.47),
      new THREE.Vector3(side * 0.58, -2.08, 0.48),
      new THREE.Vector3(side * 0.36, -2.36, 0.49),
      new THREE.Vector3(0, -2.52, 0.5),
    ], COLORS.dim, 0.74);
    lowerScroll.userData.shelfId = shelfId;
    parent.add(lowerScroll);

    const titleRosette = new THREE.Mesh(
      new THREE.TorusGeometry(0.11, 0.025, 4, 8),
      trim,
    );
    titleRosette.position.set(side * 1.13, 1.7, 0.62);
    titleRosette.rotation.z = Math.PI / 8;
    titleRosette.userData.shelfId = shelfId;
    parent.add(titleRosette);
    pickables.push(titleRosette);
  }

  const lowerBoss = new THREE.Mesh(new THREE.OctahedronGeometry(0.23, 0), trim);
  lowerBoss.position.set(0, -2.24, 0.24);
  lowerBoss.scale.set(1.35, 0.75, 0.62);
  lowerBoss.rotation.z = Math.PI / 4;
  lowerBoss.userData.shelfId = shelfId;
  parent.add(lowerBoss);
  pickables.push(lowerBoss);

  const lowerFinial = new THREE.Mesh(new THREE.ConeGeometry(0.22, 0.74, 4), trim);
  lowerFinial.position.set(0, -2.58, 0.06);
  lowerFinial.rotation.y = Math.PI / 4;
  lowerFinial.userData.shelfId = shelfId;
  parent.add(lowerFinial);
  pickables.push(lowerFinial);

  for (const x of [-0.7, -0.35, 0.35, 0.7]) {
    const dentil = new THREE.Mesh(new THREE.BoxGeometry(0.13, 0.13, 0.22), trim);
    dentil.position.set(x, -2.03 - Math.abs(x) * 0.08, 0.43);
    dentil.rotation.z = x * 0.16;
    parent.add(dentil);
  }
}

function baroqueCurve(
  points: THREE.Vector3[],
  color: number,
  opacity: number,
): THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial> {
  const curve = new THREE.CatmullRomCurve3(points, false, "centripetal", 0.42);
  const geometry = new THREE.BufferGeometry().setFromPoints(curve.getPoints(36));
  return new THREE.Line(
    geometry,
    new THREE.LineBasicMaterial({ color, transparent: true, opacity }),
  );
}

function drawShelfBooks(
  parent: THREE.Group,
  shelf: VisualShelf,
  shelfIndex: number,
  contentMaterials: THREE.Material[],
  shelfPickables: THREE.Object3D[],
  volumePickables: THREE.Object3D[],
): VolumeVisual[] {
  const volumes = volumesForShelf(shelf);
  const visuals: VolumeVisual[] = [];
  const rowBases = [0.99, 0.09, -0.81];
  const slotsPerRow = 7;
  const totalSlots = rowBases.length * slotsPerRow;
  const bookColors = [0x6f6171, 0x8e788f, 0x554d5c, 0x9c829d, 0x756a7a];

  for (let slot = 0; slot < totalSlots; slot += 1) {
    const row = Math.floor(slot / slotsPerRow);
    const column = slot % slotsPerRow;
    const volume = volumes[slot];
    const height = 0.61 + ((slot * 7 + shelfIndex * 3) % 4) * 0.045;
    const width = 0.245 + ((slot + shelfIndex) % 3) * 0.018;
    const material = new THREE.MeshStandardMaterial({
      color: bookColors[(slot + shelfIndex) % bookColors.length] ?? bookColors[0],
      emissive: volume ? 0x211829 : 0x151218,
      emissiveIntensity: volume ? 1.05 : 0.82,
      roughness: 0.72,
      metalness: 0.16,
      flatShading: true,
      transparent: true,
    });
    const geometry = new THREE.BoxGeometry(width, height, 0.38);
    const book = new THREE.Mesh(geometry, material);
    const rowBase = rowBases[row] ?? 0;
    book.position.set(-0.91 + column * 0.3, rowBase + height / 2 + 0.045, 0.35);
    book.rotation.z = column % 4 === 3 ? -0.025 : column % 5 === 4 ? 0.02 : 0;
    book.userData.shelfId = shelf.id;
    parent.add(book);
    shelfPickables.push(book);
    contentMaterials.push(material);

    const edgeMaterial = new THREE.LineBasicMaterial({
      color: volume ? COLORS.paper : COLORS.dim,
      transparent: true,
      opacity: volume ? 0.82 : 0.5,
    });
    const edges = new THREE.LineSegments(new THREE.EdgesGeometry(geometry), edgeMaterial);
    edges.position.copy(book.position);
    edges.rotation.copy(book.rotation);
    parent.add(edges);
    contentMaterials.push(edgeMaterial);

    const roman = volume ? romanFromVolume(volume) : "";
    const label = labelPlane(roman, width * 0.9, 0.29, 92, COLORS.bright, 0x09080b);
    label.position.set(book.position.x, book.position.y + height * 0.2, 0.548);
    label.rotation.copy(book.rotation);
    label.userData.shelfId = shelf.id;
    label.visible = volume !== undefined;
    if (volume) {
      label.userData.volume = volume;
      book.userData.volume = volume;
      volumePickables.push(book, label);
    }
    parent.add(label);
    contentMaterials.push(label.material);
    decorateBookSpine(
      parent,
      book,
      width,
      height,
      volume !== undefined,
      contentMaterials,
    );

    if (volume) {
      visuals.push({
        volume,
        mesh: book,
        labelMaterial: label.material,
      });
    } else {
      const chain = createVolumeChain(width, height);
      chain.position.copy(book.position);
      chain.rotation.copy(book.rotation);
      chain.userData.shelfId = shelf.id;
      parent.add(chain);
      const chainMaterials: THREE.Material[] = [];
      chain.traverse((child) => {
        if (child instanceof THREE.LineSegments || child instanceof THREE.Mesh) {
          chainMaterials.push(child.material as THREE.Material);
        }
      });
      contentMaterials.push(...chainMaterials);
    }
  }

  for (let row = 0; row < rowBases.length; row += 1) {
    const category = shelf.categories[row % Math.max(shelf.categories.length, 1)];
    const rowLabel = labelPlane(
      category?.note_type ?? "ARCHIVE",
      1.62,
      0.13,
      48,
      COLORS.paper,
      0x070708,
    );
    rowLabel.position.set(0, (rowBases[row] ?? 0) + 0.015, 0.54);
    rowLabel.userData.shelfId = shelf.id;
    parent.add(rowLabel);
    shelfPickables.push(rowLabel);
    contentMaterials.push(rowLabel.material);
  }
  return visuals;
}

function decorateBookSpine(
  parent: THREE.Group,
  book: THREE.Mesh,
  width: number,
  height: number,
  available: boolean,
  contentMaterials: THREE.Material[],
): void {
  const group = new THREE.Group();
  group.position.copy(book.position);
  group.rotation.copy(book.rotation);
  const material = new THREE.MeshBasicMaterial({
    color: available ? COLORS.brass : 0x5d5260,
    transparent: true,
    opacity: available ? 0.86 : 0.5,
    toneMapped: false,
  });
  for (const y of [-height * 0.39, height * 0.39]) {
    const band = new THREE.Mesh(
      new THREE.BoxGeometry(width * 1.04, 0.036, 0.026),
      material,
    );
    band.position.set(0, y, 0.206);
    group.add(band);
  }
  const rails = new THREE.LineSegments(
    new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-width * 0.29, -height * 0.27, 0.221),
      new THREE.Vector3(-width * 0.29, height * 0.27, 0.221),
      new THREE.Vector3(width * 0.29, -height * 0.27, 0.221),
      new THREE.Vector3(width * 0.29, height * 0.27, 0.221),
    ]),
    material,
  );
  group.add(rails);
  const boss = new THREE.Mesh(new THREE.OctahedronGeometry(0.032, 0), material);
  boss.position.set(0, -height * 0.18, 0.23);
  boss.rotation.z = Math.PI / 4;
  boss.scale.set(0.8, 1.2, 0.55);
  group.add(boss);
  parent.add(group);
  contentMaterials.push(material);
}

function createVolumeChain(width: number, height: number): THREE.Group {
  const chain = new THREE.Group();
  const material = new THREE.LineBasicMaterial({
    color: COLORS.brass,
    transparent: true,
    opacity: 0.84,
  });
  const halfWidth = width * 0.54;
  const lower = -height * 0.32;
  const upper = -height * 0.02;
  const links = new THREE.LineSegments(
    new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-halfWidth, lower, 0.215),
      new THREE.Vector3(halfWidth, upper, 0.215),
      new THREE.Vector3(-halfWidth, upper, 0.215),
      new THREE.Vector3(halfWidth, lower, 0.215),
      new THREE.Vector3(-halfWidth, (lower + upper) / 2, 0.216),
      new THREE.Vector3(halfWidth, (lower + upper) / 2, 0.216),
    ]),
    material,
  );
  chain.add(links);
  const claspMaterial = new THREE.MeshBasicMaterial({
    color: COLORS.brass,
    transparent: true,
    opacity: 0.92,
  });
  const clasp = new THREE.Mesh(new THREE.OctahedronGeometry(0.035, 0), claspMaterial);
  clasp.position.set(0, (lower + upper) / 2, 0.225);
  clasp.scale.set(0.85, 1.25, 0.4);
  chain.add(clasp);
  return chain;
}

function createAimGlow(parent: THREE.Group): {
  sprite: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>;
  material: THREE.MeshBasicMaterial;
  light: THREE.PointLight;
} {
  const material = new THREE.MeshBasicMaterial({
    map: perimeterGlowTexture(),
    color: COLORS.purpleBright,
    transparent: true,
    opacity: 0,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    side: THREE.DoubleSide,
    toneMapped: false,
  });
  const sprite = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), material);
  sprite.position.set(0, -0.05, -0.5);
  sprite.scale.set(3.75, 5.35, 1);
  sprite.visible = false;
  parent.add(sprite);
  const light = new THREE.PointLight(COLORS.purple, 0, 3.8, 2.2);
  light.position.set(0, 0, -0.48);
  parent.add(light);
  return { sprite, material, light };
}

function createStellarDial(parent: THREE.Group): StellarDial {
  const root = new THREE.Group();
  root.position.set(0, 0.05, -0.58);
  root.scale.setScalar(0.001);
  root.visible = false;
  const materials: THREE.Material[] = [];
  const spirals: StellarDialLayer[] = [];
  for (const [radius, turns, start, duration, spin, color] of [
    [2.42, 2.8, 0, 0.72, 1.25, COLORS.purpleBright],
    [2.18, 2.35, 0.12, 0.68, -0.82, COLORS.paper],
    [1.88, 1.95, 0.25, 0.62, 0.56, COLORS.dim],
  ] as const) {
    const line = spiralLine(radius, turns, color, 0);
    root.add(line);
    materials.push(line.material);
    spirals.push({
      line,
      pointCount: line.geometry.getAttribute("position").count,
      start,
      duration,
      spin,
    });
  }
  for (const [radius, color, opacity] of [
    [2.42, COLORS.paper, 0.62],
    [2.18, COLORS.purpleBright, 0.68],
    [1.9, COLORS.dim, 0.54],
  ] as const) {
    const ring = circleLine(radius, color, opacity, 96);
    root.add(ring);
    materials.push(ring.material);
  }
  const star = starLine(2.24, 1.68, 12, COLORS.paper, 0.48);
  root.add(star);
  materials.push(star.material);
  const constellation = lineSegments(
    [
      -2.12, 0.34, 0, -0.9, 1.62, 0,
      -0.9, 1.62, 0, 0.37, 2.02, 0,
      0.37, 2.02, 0, 1.96, 0.69, 0,
      1.96, 0.69, 0, 1.38, -1.09, 0,
      1.38, -1.09, 0, -0.29, -1.99, 0,
      -0.29, -1.99, 0, -2.06, -0.82, 0,
      -2.06, -0.82, 0, -2.12, 0.34, 0,
    ],
    COLORS.purpleBright,
    0.52,
  );
  root.add(constellation);
  materials.push(constellation.material);

  for (let index = 0; index < 32; index += 1) {
    const angle = (index / 32) * Math.PI * 2;
    const marker = new THREE.Mesh(
      new THREE.OctahedronGeometry(index % 6 === 0 ? 0.065 : 0.035, 0),
      new THREE.MeshBasicMaterial({
        color: index % 4 === 0 ? COLORS.purpleBright : COLORS.paper,
        transparent: true,
        opacity: 0.72,
      }),
    );
    marker.position.set(Math.cos(angle) * 2.31, Math.sin(angle) * 2.31, 0.01);
    marker.rotation.z = angle;
    root.add(marker);
    materials.push(marker.material);
  }

  const pointers = [1.76, 1.39, 1.05].map((length, index) => {
    const pointer = new THREE.Group();
    const material = new THREE.LineBasicMaterial({
      color: index === 1 ? COLORS.purpleBright : COLORS.paper,
      transparent: true,
      opacity: 0.78,
    });
    const geometry = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(0, -0.08, 0.04),
      new THREE.Vector3(0, length, 0.04),
      new THREE.Vector3(-0.09, length - 0.15, 0.04),
      new THREE.Vector3(0, length, 0.04),
      new THREE.Vector3(0.09, length - 0.15, 0.04),
    ]);
    pointer.add(new THREE.Line(geometry, material));
    const hub = new THREE.Mesh(
      new THREE.OctahedronGeometry(0.075 - index * 0.012, 0),
      new THREE.MeshBasicMaterial({ color: COLORS.purpleBright }),
    );
    hub.position.z = 0.05;
    pointer.add(hub);
    materials.push(material, hub.material);
    root.add(pointer);
    return pointer;
  });
  parent.add(root);
  return { root, materials, pointers, spirals };
}

function spiralLine(
  radius: number,
  turns: number,
  color: number,
  opacity: number,
): THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial> {
  const pointCount = 180;
  const points = Array.from({ length: pointCount }, (_, index) => {
    const progress = index / (pointCount - 1);
    const angle = progress * Math.PI * 2 * turns;
    const distance = radius * Math.pow(progress, 0.82);
    return new THREE.Vector3(
      Math.cos(angle) * distance,
      Math.sin(angle) * distance,
      progress * 0.018,
    );
  });
  return new THREE.Line(
    new THREE.BufferGeometry().setFromPoints(points),
    new THREE.LineBasicMaterial({ color, transparent: true, opacity }),
  );
}

function createSealHousing(parent: THREE.Group): {
  housing: THREE.Group;
  leaves: [THREE.Group, THREE.Group];
} {
  const ringRadius = 1.18;
  const pillarMountX = 1.18;
  const housing = new THREE.Group();
  housing.position.set(0, -0.18, 0.62);
  housing.scale.setScalar(SEAL_HOUSING_SCALE);
  const metal = new THREE.MeshStandardMaterial({
    color: 0x6b626f,
    emissive: 0x211824,
    emissiveIntensity: 0.82,
    roughness: 0.46,
    metalness: 0.78,
    flatShading: true,
  });
  const darkMetal = new THREE.MeshStandardMaterial({
    color: 0x29232e,
    emissive: 0x140d18,
    emissiveIntensity: 0.78,
    roughness: 0.6,
    metalness: 0.68,
    flatShading: true,
  });

  const leaves = [-1, 1].map((side) => {
    const pivot = new THREE.Group();
    pivot.position.x = side * pillarMountX;
    const mountingPlate = new THREE.Mesh(
      new THREE.BoxGeometry(0.16, 0.62, 0.24),
      darkMetal,
    );
    mountingPlate.position.z = -0.02;
    pivot.add(mountingPlate);
    for (const y of [-0.23, 0.23]) {
      const bracket = new THREE.Mesh(
        new THREE.BoxGeometry(0.25, 0.1, 0.3),
        metal,
      );
      bracket.position.set(-side * 0.035, y, 0.015);
      pivot.add(bracket);
    }
    const outerRing = new THREE.Mesh(
      new THREE.TorusGeometry(ringRadius, 0.095, 6, 40, Math.PI),
      metal,
    );
    outerRing.position.x = -side * pillarMountX;
    outerRing.position.z = 0.02;
    outerRing.rotation.z = side < 0 ? Math.PI / 2 : -Math.PI / 2;
    pivot.add(outerRing);
    const innerRing = new THREE.Mesh(
      new THREE.TorusGeometry(0.98, 0.03, 4, 36, Math.PI),
      darkMetal,
    );
    innerRing.position.x = -side * pillarMountX;
    innerRing.position.z = 0.045;
    innerRing.rotation.z = side < 0 ? Math.PI / 2 : -Math.PI / 2;
    pivot.add(innerRing);

    for (const angle of [-1.18, -0.64, 0, 0.64, 1.18]) {
      const ringAngle = side < 0 ? Math.PI - angle : angle;
      const ornament = new THREE.Mesh(
        new THREE.OctahedronGeometry(Math.abs(angle) < 0.1 ? 0.075 : 0.052, 0),
        angle === 0 ? metal : darkMetal,
      );
      ornament.position.set(
        Math.cos(ringAngle) * ringRadius - side * pillarMountX,
        Math.sin(ringAngle) * ringRadius,
        0.12,
      );
      ornament.rotation.z = ringAngle + Math.PI / 4;
      ornament.scale.set(0.7, Math.abs(angle) < 0.1 ? 1.45 : 1.1, 0.55);
      pivot.add(ornament);
    }

    const hinge = new THREE.Mesh(new THREE.CylinderGeometry(0.07, 0.07, 0.54, 6), metal);
    hinge.position.set(0, 0, -0.015);
    hinge.rotation.x = Math.PI / 2;
    pivot.add(hinge);
    for (const y of [-0.2, 0.2]) {
      const hingeCap = new THREE.Mesh(new THREE.OctahedronGeometry(0.07, 0), darkMetal);
      hingeCap.position.set(0, y, 0.015);
      pivot.add(hingeCap);
    }
    housing.add(pivot);
    return pivot;
  }) as [THREE.Group, THREE.Group];

  const crown = lineSegments(
    [
      -0.74, 1.04, 0.14, -0.4, 1.34, 0.14,
      -0.4, 1.34, 0.14, 0, 1.16, 0.14,
      0, 1.16, 0.14, 0.4, 1.34, 0.14,
      0.4, 1.34, 0.14, 0.74, 1.04, 0.14,
      -0.74, -1.04, 0.14, -0.4, -1.34, 0.14,
      -0.4, -1.34, 0.14, 0, -1.16, 0.14,
      0, -1.16, 0.14, 0.4, -1.34, 0.14,
      0.4, -1.34, 0.14, 0.74, -1.04, 0.14,
    ],
    COLORS.dim,
    0.58,
  );
  housing.add(crown);
  parent.add(housing);
  return { housing, leaves };
}

function createSeal(
  parent: THREE.Group,
  symbol: PixelSymbol,
  random: () => number,
  stones: ShelfMagicStone[],
): SealVisual {
  const lock = createSealHousing(parent);
  const root = new THREE.Group();
  root.position.set(0, 0, 0.22);
  const glowMaterial = new THREE.SpriteMaterial({
    map: radialGlowTexture(),
    color: COLORS.purpleBright,
    transparent: true,
    opacity: 0,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const glow = new THREE.Sprite(glowMaterial);
  glow.scale.set(3.05, 3.05, 1);
  glow.position.z = -0.06;
  root.add(glow);

  const poolMaterial = new THREE.MeshBasicMaterial({
    color: 0x70408b,
    transparent: true,
    opacity: 0.2,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    side: THREE.DoubleSide,
    toneMapped: false,
  });
  const pool = new THREE.Mesh(new THREE.CircleGeometry(0.96, 56), poolMaterial);
  pool.position.z = 0.015;
  root.add(pool);

  const sigil = new THREE.Group();
  sigil.position.z = 0.11;
  const sigilMaterials: THREE.Material[] = [];
  const symbolPixels = symbol.rows.flatMap((row, rowIndex) =>
    [...row].flatMap((value, columnIndex) =>
      value === "#"
        ? [{
          x: (columnIndex - (row.length - 1) / 2) * 0.105,
          y: ((symbol.rows.length - 1) / 2 - rowIndex) * 0.105,
        }]
        : [],
    ),
  );
  const sigilMaterial = new THREE.MeshBasicMaterial({
    color: COLORS.paper,
    transparent: true,
    opacity: 0.9,
    toneMapped: false,
  });
  const sigilPixels = new THREE.InstancedMesh(
    new THREE.BoxGeometry(0.068, 0.068, 0.045),
    sigilMaterial,
    symbolPixels.length,
  );
  const matrix = new THREE.Matrix4();
  symbolPixels.forEach((pixel, index) => {
    matrix.makeTranslation(pixel.x, pixel.y, 0);
    sigilPixels.setMatrixAt(index, matrix);
  });
  sigil.add(sigilPixels);
  sigilMaterials.push(sigilMaterial);
  root.add(sigil);

  const unlock = new THREE.Group();
  unlock.position.z = 0.13;
  const unlockMaterials: THREE.Material[] = [];
  for (const [radius, color] of [
    [0.91, COLORS.purpleBright],
    [0.72, COLORS.paper],
  ] as const) {
    const ring = circleLine(radius, color, 0, 72);
    unlock.add(ring);
    unlockMaterials.push(ring.material);
  }
  const unlockStar = starLine(0.88, 0.7, 8, COLORS.purpleBright, 0);
  unlock.add(unlockStar);
  unlockMaterials.push(unlockStar.material);
  root.add(unlock);

  const particleCount = 192;
  const particleOrigins = new Float32Array(particleCount * 3);
  const particleVelocities = new Float32Array(particleCount * 3);
  const particleTargets = new Float32Array(particleCount * 3);
  const particlePhases = new Float32Array(particleCount);
  const fallbackTargets = [
    new THREE.Vector3(-1.02, -2.48, -0.05),
    new THREE.Vector3(1.02, -2.48, -0.05),
    new THREE.Vector3(0, 2.34, 0.1),
  ];
  const crownStones = stones.filter((stone) => stone.role === "crown");
  const modelTargets = crownStones.length > 0
    ? crownStones.map((stone) => stone.mesh.position.clone())
    : fallbackTargets;
  const stoneTargets = modelTargets.map((target) =>
    target
      .sub(lock.housing.position)
      .divideScalar(SEAL_HOUSING_SCALE)
      .sub(root.position),
  );
  for (let index = 0; index < particleCount; index += 1) {
    const angle = random() * Math.PI * 2;
    const radius = Math.sqrt(random()) * 0.91;
    particleOrigins[index * 3] = Math.cos(angle) * radius;
    particleOrigins[index * 3 + 1] = Math.sin(angle) * radius;
    particleOrigins[index * 3 + 2] = 0.12;
    particleVelocities[index * 3] = Math.cos(angle + Math.PI / 2) * (0.24 + random() * 0.42);
    particleVelocities[index * 3 + 1] = Math.sin(angle + Math.PI / 2) * (0.24 + random() * 0.42);
    particleVelocities[index * 3 + 2] = 0.08 + random() * 0.26;
    const target = stoneTargets[index % stoneTargets.length] ?? stoneTargets[0]!;
    particleTargets[index * 3] = target.x + (random() - 0.5) * 0.12;
    particleTargets[index * 3 + 1] = target.y + (random() - 0.5) * 0.12;
    particleTargets[index * 3 + 2] = target.z + (random() - 0.5) * 0.1;
    particlePhases[index] = random();
  }
  const particleGeometry = new THREE.BufferGeometry();
  particleGeometry.setAttribute("position", new THREE.BufferAttribute(particleOrigins.slice(), 3));
  const particleMaterial = new THREE.PointsMaterial({
    color: COLORS.purpleBright,
    size: 0.045,
    sizeAttenuation: true,
    transparent: true,
    opacity: 0,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const particles = new THREE.Points(particleGeometry, particleMaterial);
  root.add(particles);
  const energyStreams: SealEnergyStream[] = [];
  stoneTargets.forEach((target, targetIndex) => {
    for (let strand = 0; strand < 3; strand += 1) {
      const phase = (strand + targetIndex * 0.37) / 3;
      const startAngle = (targetIndex / Math.max(stoneTargets.length, 1)) * Math.PI * 2 +
        strand * 0.74;
      const start = new THREE.Vector3(
        Math.cos(startAngle) * (0.22 + strand * 0.09),
        Math.sin(startAngle) * (0.22 + strand * 0.09),
        0.16 + strand * 0.012,
      );
      const line = energyStreamLine(start, target, phase, strand === 1 ? COLORS.paper : COLORS.purpleBright);
      root.add(line);
      energyStreams.push({
        line,
        material: line.material,
        pointCount: line.geometry.getAttribute("position").count,
        phase,
      });
    }
  });
  lock.housing.add(root);
  return {
    housing: lock.housing,
    lockLeaves: lock.leaves,
    root,
    pool,
    poolMaterial,
    sigil,
    sigilMaterials,
    glow,
    glowMaterial,
    unlock,
    unlockMaterials,
    particles,
    particleOrigins,
    particleVelocities,
    particleTargets,
    particlePhases,
    energyStreams,
    stones,
    lastProgress: 0,
  };
}

function energyStreamLine(
  start: THREE.Vector3,
  target: THREE.Vector3,
  phase: number,
  color: number,
): THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial> {
  const pointCount = 52;
  const points = Array.from({ length: pointCount }, (_, index) => {
    const progress = index / (pointCount - 1);
    const eased = progress * progress * (3 - 2 * progress);
    const point = start.clone().lerp(target, eased);
    const curl = Math.sin(progress * Math.PI) * (0.22 + phase * 0.12);
    point.x += Math.cos(progress * Math.PI * 3.2 + phase * Math.PI * 2) * curl;
    point.y += Math.sin(progress * Math.PI * 2.6 + phase * Math.PI * 2) * curl * 0.72;
    point.z += Math.sin(progress * Math.PI) * (0.18 + phase * 0.08);
    return point;
  });
  const material = new THREE.LineBasicMaterial({
    color,
    transparent: true,
    opacity: 0,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const line = new THREE.Line(new THREE.BufferGeometry().setFromPoints(points), material);
  line.geometry.setDrawRange(0, 0);
  return line;
}

function updateFreeMotion(actors: ShelfActor[], seconds: number): void {
  const freeActors = actors.filter(
    (actor) => !actor.dragging && actor.focusProgress < 0.08 && actor.focusTarget === 0,
  );
  for (const actor of freeActors) {
    actor.motion = advanceShelfMotion(actor.motion, seconds, actor.cell);
    actor.homePosition.copy(motionToWorld(actor.motion));
  }
}

function updateActors(
  actors: ShelfActor[],
  camera: THREE.PerspectiveCamera,
  aimedId: string | null,
  focusedId: string | null,
  openedVolumeId: string | null,
  selectedBookId: string | null,
  elapsed: number,
  seconds: number,
  reducedMotion: boolean,
): void {
  const focusPresence = actors.reduce(
    (maximum, actor) => Math.max(maximum, smoothstep(0, 1, actor.focusProgress)),
    0,
  );
  const freePosition = new THREE.Vector3();
  const orbitPosition = new THREE.Vector3();
  for (const actor of actors) {
    if (reducedMotion) {
      actor.focusProgress = actor.focusTarget;
      actor.sealProgress = actor.focusTarget;
      actor.dialProgress = actor.focusTarget;
    } else if (actor.focusTarget === 1) {
      actor.focusProgress = moveTowards(actor.focusProgress, 1, seconds / 1.15);
      if (actor.focusProgress > 0.55) {
        actor.sealProgress = moveTowards(actor.sealProgress, 1, seconds / 0.95);
      }
    } else {
      actor.sealProgress = moveTowards(actor.sealProgress, 0, seconds / 1.45);
      if (actor.sealProgress < 0.1) {
        actor.focusProgress = moveTowards(actor.focusProgress, 0, seconds / 1.25);
      }
    }

    if (!reducedMotion) {
      const dialTarget = stellarDialTarget(
        actor.sealProgress,
        actor.focusTarget === 1,
      );
      actor.dialProgress = moveTowards(
        actor.dialProgress,
        dialTarget,
        seconds / (dialTarget === 1 ? 1.65 : 2.6),
      );
    }

    const focus = smoothstep(0, 1, actor.focusProgress);
    if (
      !reducedMotion &&
      !actor.dragging &&
      actor.focusTarget === 0 &&
      actor.focusProgress < 0.08
    ) {
      const orbitTarget = edgeShelfOrbitTarget(actor.motion.x);
      const orbitResponse = 1 - Math.exp(-1.15 * seconds);
      actor.orbitYaw = mix(actor.orbitYaw, orbitTarget, orbitResponse);
    }
    if (!actor.dragging) {
      const orbit = orbitAroundGroundAxis(
        actor.homePosition.x,
        actor.homePosition.z,
        actor.orbitYaw,
        GROUND_ORBIT_CENTER.x,
        GROUND_ORBIT_CENTER.z,
      );
      orbitPosition.set(orbit.x, actor.homePosition.y, orbit.z);
      freePosition.copy(orbitPosition);
      if (focusedId && actor.shelf.id !== focusedId) {
        const side = Math.sign(freePosition.x) || (actor.phase > Math.PI ? 1 : -1);
        const clearance = Math.max(
          0,
          VISUAL_CONTRACT.focus.inactiveCenterClearance - Math.abs(freePosition.x),
        );
        freePosition.x += side * clearance * focusPresence;
        freePosition.z -= VISUAL_CONTRACT.focus.inactiveDepthOffset * focusPresence;
      }
      actor.root.position.lerpVectors(freePosition, FOCUS_POSITION, focus);
    }
    const freeScale = (0.79 + (1 - actor.motion.z) * 0.1) * actor.layoutScale;
    const scale = mix(freeScale, VISUAL_CONTRACT.focus.scale, focus);
    actor.root.scale.setScalar(scale);
    actor.root.rotation.x = mix(
      Math.sin(elapsed * 0.34 + actor.phase) * 0.035,
      0,
      focus,
    );
    actor.root.rotation.y = mix(
      actor.orbitYaw + Math.sin(elapsed * 0.27 + actor.phase * 1.7) * 0.055,
      0,
      focus,
    );
    actor.root.rotation.z = mix(
      Math.sin(elapsed * 0.22 + actor.phase * 0.6) * 0.014,
      0,
      focus,
    );

    const aimed = actor.shelf.id === aimedId && actor.shelf.id !== focusedId;
    const aimTarget = aimed ? 1 : 0;
    actor.aimProgress = reducedMotion
      ? aimTarget
      : mix(actor.aimProgress, aimTarget, 1 - Math.exp(-2.4 * seconds));
    actor.aimGlow.visible = actor.aimProgress > 0.002;
    actor.aimGlowMaterial.opacity = actor.aimProgress * 0.58;
    actor.aimGlow.scale.set(
      3.48 + actor.aimProgress * 0.34,
      5.02 + actor.aimProgress * 0.42,
      1,
    );
    actor.aimLight.intensity = actor.aimProgress * 0.72;
    actor.focusLight.intensity = focus * 2.35;
    actor.cabinetMaterials.wood.emissiveIntensity = mix(0.95, 1.2, focus);
    actor.cabinetMaterials.trim.emissiveIntensity = mix(0.82, 1.08, focus);
    actor.cabinetMaterials.dark.emissiveIntensity = mix(0.82, 1.05, focus);

    updateDial(actor.dial, actor.dialProgress, elapsed, reducedMotion);
    updateSeal(actor.seal, actor.sealProgress, elapsed);
    const contentOpacity = mix(0.72, 1, focus);
    for (const material of actor.contentMaterials) {
      setOpacity(material, contentOpacity);
    }
    for (const volume of actor.volumes) {
      const selected = selectedBookId !== null &&
        volume.volume.books.some((book) => book.id === selectedBookId);
      if (selected) {
        volume.mesh.material.emissive.setHex(COLORS.purpleBright);
        volume.mesh.material.emissiveIntensity = 0.58;
      } else if (volume.volume.id === openedVolumeId) {
        volume.mesh.material.emissive.setHex(COLORS.purple);
        volume.mesh.material.emissiveIntensity = 0.42;
      } else {
        volume.mesh.material.emissive.setHex(0x0b090d);
        volume.mesh.material.emissiveIntensity = 1;
      }
      volume.labelMaterial.color.setHex(
        selected || volume.volume.id === openedVolumeId ? COLORS.purpleBright : COLORS.paper,
      );
    }
    actor.projected = projectWithCamera(actor.shelf.id, actor.root.position, camera);
  }
}

function updateDial(
  dial: StellarDial,
  dialProgress: number,
  elapsed: number,
  reducedMotion: boolean,
): void {
  const expansion = smoothstep(0, 1, dialProgress);
  dial.root.visible = expansion > 0.002;
  dial.root.scale.setScalar(Math.max(0.12 + expansion * 0.88, 0.001));
  dial.root.rotation.z = reducedMotion
    ? expansion * Math.PI * 1.75
    : expansion * Math.PI * 1.75 + elapsed * 0.032;
  const ranges = [68, 52, 79];
  dial.pointers.forEach((pointer, index) => {
    const range = THREE.MathUtils.degToRad(ranges[index] ?? 60);
    const speed = [0.24, 0.17, 0.11][index] ?? 0.2;
    pointer.rotation.z = Math.sin(elapsed * speed + index * 1.9) * range;
  });
  for (const material of dial.materials) {
    setOpacity(material, expansion * 0.78);
  }
  dial.spirals.forEach((layer) => {
    const reveal = smoothstep(
      layer.start,
      Math.min(1, layer.start + layer.duration),
      dialProgress,
    );
    layer.line.geometry.setDrawRange(0, Math.max(1, Math.floor(layer.pointCount * reveal)));
    layer.line.rotation.z = reducedMotion
      ? reveal * layer.spin
      : reveal * layer.spin + elapsed * 0.014 * Math.sign(layer.spin);
    setOpacity(layer.line.material, reveal * 0.92);
  });
}

function updateSeal(seal: SealVisual, progress: number, elapsed: number): void {
  const frame = sealFrame(progress);
  const reversing = progress < seal.lastProgress;
  const transformation = smoothstep(0.26, 0.78, progress);
  const membraneAlpha = 1 - smoothstep(0.3, 0.78, progress);
  seal.root.visible = membraneAlpha > 0.001 || frame.particleAlpha > 0.001 || frame.circleAlpha > 0.001;
  const lockOpen = smoothstep(0.72, 0.92, progress);
  seal.lockLeaves[0].rotation.y = -lockOpen * 1.25;
  seal.lockLeaves[1].rotation.y = lockOpen * 1.25;
  seal.poolMaterial.opacity = membraneAlpha * (0.42 + frame.glowAlpha * 0.1);
  seal.pool.scale.setScalar(1 - transformation * 0.26);
  seal.pool.rotation.z = elapsed * 0.006;
  seal.sigil.position.z = 0.11 + transformation * 0.1;
  seal.sigil.scale.setScalar(Math.max(0.035, 1 - transformation * 0.96));
  seal.sigil.rotation.z = transformation * 0.32;
  for (const material of seal.sigilMaterials) {
    setOpacity(material, membraneAlpha * 0.92);
  }
  seal.glowMaterial.opacity = membraneAlpha * 0.12 + frame.glowAlpha * 0.34;
  seal.unlock.visible = frame.circleAlpha > 0.001;
  seal.unlock.rotation.z = progress * Math.PI * 1.35;
  seal.unlock.scale.setScalar(0.72 + smoothstep(0, 0.4, progress) * 0.34);
  for (const material of seal.unlockMaterials) {
    setOpacity(material, frame.circleAlpha * 0.72);
  }
  seal.particles.material.opacity = frame.particleAlpha;
  const positions = seal.particles.geometry.getAttribute("position") as THREE.BufferAttribute;
  const values = positions.array as Float32Array;
  for (let index = 0; index < values.length; index += 3) {
    const particleIndex = index / 3;
    const phase = seal.particlePhases[particleIndex] ?? 0;
    const travel = smoothstep(0.48 + phase * 0.16, 0.97, progress);
    const curl = Math.sin(travel * Math.PI) * (0.72 + phase * 0.42);
    values[index] = mix(
      seal.particleOrigins[index] ?? 0,
      seal.particleTargets[index] ?? 0,
      travel,
    ) + (seal.particleVelocities[index] ?? 0) * curl;
    values[index + 1] = mix(
      seal.particleOrigins[index + 1] ?? 0,
      seal.particleTargets[index + 1] ?? 0,
      travel,
    ) + (seal.particleVelocities[index + 1] ?? 0) * curl;
    values[index + 2] = mix(
      seal.particleOrigins[index + 2] ?? 0,
      seal.particleTargets[index + 2] ?? 0,
      travel,
    ) + (seal.particleVelocities[index + 2] ?? 0) * curl;
  }
  positions.needsUpdate = true;

  const flowProgress = smoothstep(0.36, 0.96, progress);
  seal.energyStreams.forEach((stream) => {
    const strandProgress = clamp(flowProgress * 1.22 - stream.phase * 0.22, 0, 1);
    const count = Math.max(0, Math.floor(stream.pointCount * strandProgress));
    const start = reversing ? Math.max(0, stream.pointCount - count) : 0;
    stream.line.geometry.setDrawRange(start, count);
    stream.material.opacity = frame.particleAlpha * (0.62 + stream.phase * 0.22);
  });

  const storedEnergy = smoothstep(0.68, 0.98, progress);
  const transferPosition = smoothstep(0.43, 0.97, progress);
  const transferEnergy = Math.sin(transferPosition * Math.PI);
  seal.stones.forEach((stone, index) => {
    const roleStrength = stone.role === "crown" ? 1 : stone.role === "pendant" ? 0.28 : 0.16;
    const pulse = storedEnergy > 0 && stone.role === "crown"
      ? (Math.sin(elapsed * 1.05 + stone.phase + index * 0.7) + 1) * 0.018
      : 0;
    stone.mesh.scale.copy(stone.baseScale).multiplyScalar(
      1 + storedEnergy * 0.24 * roleStrength + transferEnergy * 0.2 * roleStrength + pulse,
    );
    stone.mesh.rotation.y = elapsed * (0.035 + (index % 3) * 0.009);
    stone.material.emissive
      .copy(PURPLE_DARK_COLOR)
      .lerp(
        PURPLE_BRIGHT_COLOR,
        (storedEnergy * 0.86 + transferEnergy * 0.14) * roleStrength,
      );
    stone.material.emissiveIntensity =
      1.7 + (storedEnergy * 4.2 + transferEnergy * 2.8) * roleStrength;
  });
  seal.lastProgress = progress;
}

function addLighting(scene: THREE.Scene): void {
  scene.add(new THREE.AmbientLight(0x8d8791, 0.62));
  const key = new THREE.DirectionalLight(COLORS.paper, 1.55);
  key.position.set(-4, 7, 8);
  scene.add(key);
  const purple = new THREE.PointLight(COLORS.purple, 3.2, 17, 2);
  purple.position.set(0, -2.4, 1.5);
  scene.add(purple);
  const rim = new THREE.DirectionalLight(COLORS.purpleBright, 1.1);
  rim.position.set(5, 2, -7);
  scene.add(rim);
}

function createStarField(scene: THREE.Scene, random: () => number): THREE.Points {
  const count = 430;
  const positions = new Float32Array(count * 3);
  const colors = new Float32Array(count * 3);
  const pale = new THREE.Color(COLORS.paper);
  const purple = new THREE.Color(COLORS.purpleBright);
  for (let index = 0; index < count; index += 1) {
    positions[index * 3] = (random() - 0.5) * 25;
    positions[index * 3 + 1] = (random() - 0.42) * 13;
    positions[index * 3 + 2] = -4 - random() * 22;
    const color = random() > 0.9 ? purple : pale;
    colors[index * 3] = color.r;
    colors[index * 3 + 1] = color.g;
    colors[index * 3 + 2] = color.b;
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  const material = new THREE.PointsMaterial({
    vertexColors: true,
    size: 0.032,
    transparent: true,
    opacity: 0.8,
    sizeAttenuation: true,
    depthWrite: false,
  });
  const stars = new THREE.Points(geometry, material);
  scene.add(stars);
  return stars;
}

function createWorldMotes(scene: THREE.Scene, random: () => number): RisingPixels {
  const count = 140;
  const positions = new Float32Array(count * 3);
  const colors = new Float32Array(count * 3);
  const speeds = new Float32Array(count);
  const purple = new THREE.Color(COLORS.purpleBright);
  for (let index = 0; index < count; index += 1) {
    positions[index * 3] = (random() - 0.5) * 12;
    positions[index * 3 + 1] = -2.5 + random() * 5;
    positions[index * 3 + 2] = -1 - random() * 8;
    const fade = Math.sin(((positions[index * 3 + 1] ?? 0) + 2.5) / 5.2 * Math.PI);
    colors[index * 3] = purple.r * fade;
    colors[index * 3 + 1] = purple.g * fade;
    colors[index * 3 + 2] = purple.b * fade;
    speeds[index] = 0.045 + random() * 0.085;
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  const material = new THREE.PointsMaterial({
    vertexColors: true,
    size: 0.04,
    transparent: true,
    opacity: 0.48,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const motes = new THREE.Points(geometry, material);
  scene.add(motes);
  return { points: motes, speeds };
}

function createGroundCircle(scene: THREE.Scene): GroundSigil {
  const root = new THREE.Group();
  root.position.set(0, -2.38, -1.6);
  const material = new THREE.MeshBasicMaterial({
    map: groundSigilTexture(),
    color: COLORS.paper,
    transparent: true,
    opacity: 0.42,
    depthWrite: false,
    side: THREE.DoubleSide,
    blending: THREE.AdditiveBlending,
  });
  const base = new THREE.Mesh(new THREE.PlaneGeometry(12.5, 12.5), material);
  base.rotation.x = -Math.PI / 2;
  root.add(base);
  const echoes = [0, 1].map((index) => {
    const echoMaterial = new THREE.MeshBasicMaterial({
      map: groundEchoTexture(index),
      color: index === 0 ? COLORS.purpleBright : COLORS.paper,
      transparent: true,
      opacity: 0,
      depthWrite: false,
      side: THREE.DoubleSide,
      blending: THREE.AdditiveBlending,
    });
    const echo = new THREE.Mesh(new THREE.PlaneGeometry(12.5, 12.5), echoMaterial);
    echo.rotation.x = -Math.PI / 2;
    echo.position.y = 0.008 + index * 0.006;
    echo.scale.setScalar(0.82);
    root.add(echo);
    return echo;
  });
  scene.add(root);
  return { root, base, echoes };
}

function createBackgroundEffects(
  world: THREE.Scene,
  overlay: THREE.Scene,
  random: () => number,
): BackgroundEffects {
  const comets = Array.from({ length: 4 }, () => {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(new Float32Array(21 * 3), 3));
    const material = new THREE.LineBasicMaterial({
      color: COLORS.paper,
      transparent: true,
      opacity: 0,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const line = new THREE.Line(geometry, material);
    line.visible = false;
    line.renderOrder = -2;
    world.add(line);
    const headMaterial = new THREE.SpriteMaterial({
      map: radialGlowTexture(),
      color: COLORS.purpleBright,
      transparent: true,
      opacity: 0,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const head = new THREE.Sprite(headMaterial);
    head.scale.set(0.34, 0.34, 1);
    head.visible = false;
    head.renderOrder = -1;
    world.add(head);
    return {
      line,
      head,
      headMaterial,
      start: Number.NEGATIVE_INFINITY,
      duration: 2.6,
      from: new THREE.Vector3(),
      to: new THREE.Vector3(),
      bend: 0,
    };
  });

  const lightning = Array.from({ length: 3 }, () => {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(new Float32Array(17 * 3), 3));
    const material = new THREE.LineBasicMaterial({
      color: COLORS.purpleBright,
      transparent: true,
      opacity: 0,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const line = new THREE.Line(geometry, material);
    line.visible = false;
    line.renderOrder = -2;
    world.add(line);
    return { line, start: Number.NEGATIVE_INFINITY, duration: 0.55 };
  });

  const silhouettes = Array.from({ length: 4 }, (_, index) => {
    const material = new THREE.SpriteMaterial({
      map: mysteriousSilhouetteTexture(index),
      transparent: true,
      opacity: 0,
      depthTest: false,
      depthWrite: false,
    });
    const sprite = new THREE.Sprite(material);
    const side = index % 2 === 0 ? -1 : 1;
    const baseX = side * (420 + (index % 3) * 54);
    const baseY = -220 + (index % 2) * 22;
    sprite.position.set(baseX, baseY, 0.4);
    sprite.scale.set(145 + index * 12, 235 + index * 18, 1);
    sprite.visible = false;
    sprite.renderOrder = 2;
    overlay.add(sprite);
    return {
      sprite,
      material,
      baseX,
      baseY,
      start: Number.NEGATIVE_INFINITY,
      duration: 4,
      phase: random() * Math.PI * 2,
    };
  });

  const nebulae = Array.from({ length: 3 }, (_, index) => {
    const material = new THREE.SpriteMaterial({
      map: nebulaTexture(index),
      color: 0xffffff,
      transparent: true,
      opacity: 0,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const sprite = new THREE.Sprite(material);
    const side = index % 2 === 0 ? -1 : 1;
    const base = new THREE.Vector3(side * (4.1 + index * 0.7), 1.6 - index * 0.5, -15 - index * 2.5);
    const baseScale = new THREE.Vector2(5.8 + index * 0.8, 2.8 + index * 0.35);
    sprite.position.copy(base);
    sprite.scale.set(baseScale.x, baseScale.y, 1);
    sprite.visible = false;
    sprite.renderOrder = -3;
    world.add(sprite);
    return {
      sprite,
      material,
      start: Number.NEGATIVE_INFINITY,
      duration: 11,
      phase: random() * Math.PI * 2,
      base,
      baseScale,
    };
  });

  const apertures = Array.from({ length: 3 }, (_, index) => {
    const material = new THREE.SpriteMaterial({
      map: arcaneApertureTexture(index),
      color: index === 1 ? COLORS.paper : COLORS.purpleBright,
      transparent: true,
      opacity: 0,
      depthWrite: false,
    });
    const sprite = new THREE.Sprite(material);
    const side = index % 2 === 0 ? -1 : 1;
    const base = new THREE.Vector3(side * (5.4 + index * 0.8), 1.2 + index * 0.7, -14 - index * 3);
    sprite.position.copy(base);
    sprite.scale.set(1.45 + index * 0.14, 1.45 + index * 0.14, 1);
    sprite.visible = false;
    world.add(sprite);
    return {
      sprite,
      material,
      start: Number.NEGATIVE_INFINITY,
      duration: 8,
      phase: random() * Math.PI * 2,
      base,
    };
  });

  return { comets, lightning, silhouettes, nebulae, apertures };
}

function createCornerFog(scene: THREE.Scene): FogSprite[] {
  const fog: FogSprite[] = [];
  for (const side of [-1, 1]) {
    for (let layer = 0; layer < 4; layer += 1) {
      const baseOpacity = 0.86 - layer * 0.075;
      const material = new THREE.SpriteMaterial({
        map: cornerFogTexture(side as -1 | 1, layer),
        color: layer === 0 ? 0xd0ccd3 : 0x8f8994,
        transparent: true,
        opacity: baseOpacity,
        depthTest: false,
        depthWrite: false,
      });
      const sprite = new THREE.Sprite(material);
      const baseX = side * (230 + layer * 26);
      const baseY = -284 + layer * 18;
      sprite.position.set(baseX, baseY, 0);
      sprite.scale.set(850 - layer * 58, 370 - layer * 24, 1);
      sprite.renderOrder = 1;
      scene.add(sprite);
      fog.push({
        sprite,
        baseX,
        baseY,
        baseOpacity,
        phase: side * 1.4 + layer * 2.1,
      });
    }
  }
  return fog;
}

function updateAmbient(
  stars: THREE.Points,
  ground: GroundSigil,
  fog: FogSprite[],
  motes: RisingPixels,
  effects: BackgroundEffects,
  ambient: AmbientState,
  elapsed: number,
  seconds: number,
  reducedMotion: boolean,
  random: () => number,
): void {
  updateBackgroundEffects(effects, elapsed, reducedMotion);
  if (reducedMotion) {
    ground.base.material.opacity = 0.42;
    ground.root.rotation.y = 0;
    ground.echoes.forEach((echo) => {
      echo.material.opacity = 0;
      echo.scale.setScalar(0.82);
    });
    (stars.material as THREE.PointsMaterial).opacity = 0.82;
    (stars.material as THREE.PointsMaterial).size = 0.04;
    motes.points.material.opacity = 0.34;
    fog.forEach((item) => {
      item.sprite.position.set(item.baseX, item.baseY, 0);
      (item.sprite.material as THREE.SpriteMaterial).opacity = item.baseOpacity;
    });
    return;
  }

  const groundAge = elapsed - ambient.groundPulseStart;
  const groundPulse = groundAge >= 0 && groundAge < VISUAL_CONTRACT.effects.groundDuration
    ? Math.sin((groundAge / VISUAL_CONTRACT.effects.groundDuration) * Math.PI)
    : 0;
  ground.base.material.opacity = 0.4 + groundPulse * 0.065;
  ground.root.rotation.y = elapsed * 0.0016;
  ground.echoes.forEach((echo, index) => {
    const echoProgress = clamp(
      groundAge / VISUAL_CONTRACT.effects.groundDuration * 1.34 - index * 0.22,
      0,
      1,
    );
    echo.material.opacity = Math.sin(echoProgress * Math.PI) * (index === 0 ? 0.16 : 0.1);
    echo.scale.setScalar(0.82 + echoProgress * (0.42 + index * 0.12));
    echo.rotation.z = (index === 0 ? 1 : -1) * echoProgress * 0.12;
  });

  const flashAge = elapsed - ambient.starFlashStart;
  const flash = flashAge >= 0 && flashAge < VISUAL_CONTRACT.effects.starDuration
    ? Math.sin((flashAge / VISUAL_CONTRACT.effects.starDuration) * Math.PI)
    : 0;
  (stars.material as THREE.PointsMaterial).opacity = 0.76 + flash * 0.1;
  (stars.material as THREE.PointsMaterial).size = 0.038 + flash * 0.014;
  stars.rotation.y = elapsed * 0.0018;

  const mistAge = elapsed - ambient.mistSurgeStart;
  const mist = mistAge >= 0 && mistAge < VISUAL_CONTRACT.effects.mistDuration
    ? Math.sin((mistAge / VISUAL_CONTRACT.effects.mistDuration) * Math.PI)
    : 0;
  fog.forEach((item) => {
    item.sprite.position.x = item.baseX + Math.sin(elapsed * 0.1 + item.phase) * 24;
    item.sprite.position.y = item.baseY + Math.sin(elapsed * 0.16 + item.phase) * 9;
    (item.sprite.material as THREE.SpriteMaterial).opacity =
      Math.min(0.92, item.baseOpacity + mist * 0.1);
  });

  const position = motes.points.geometry.getAttribute("position") as THREE.BufferAttribute;
  const color = motes.points.geometry.getAttribute("color") as THREE.BufferAttribute;
  const values = position.array as Float32Array;
  const colorValues = color.array as Float32Array;
  for (let index = 0; index < motes.speeds.length; index += 1) {
    const yIndex = index * 3 + 1;
    values[yIndex] = (values[yIndex] ?? 0) + seconds * (motes.speeds[index] ?? 0.06);
    if ((values[yIndex] ?? 0) > 2.7) {
      values[yIndex] = -2.5;
    }
    const age = clamp(((values[yIndex] ?? 0) + 2.5) / 5.2, 0, 1);
    const fade = Math.pow(Math.sin(age * Math.PI), 1.35);
    colorValues[index * 3] = PURPLE_BRIGHT_COLOR.r * fade;
    colorValues[index * 3 + 1] = PURPLE_BRIGHT_COLOR.g * fade;
    colorValues[index * 3 + 2] = PURPLE_BRIGHT_COLOR.b * fade;
  }
  position.needsUpdate = true;
  color.needsUpdate = true;
  motes.points.material.opacity = 0.42 + mist * 0.14;

  if (elapsed >= ambient.nextMinorAt) {
    const pattern = Math.floor(random() * 8);
    if (pattern === 0) {
      spawnNebula(effects, elapsed, random);
    } else if (pattern === 1) {
      spawnAperture(effects, elapsed, random);
    } else if (pattern === 2) {
      spawnSilhouette(effects, elapsed, random);
    } else if (pattern === 3) {
      ambient.mistSurgeStart = elapsed;
      spawnNebula(effects, elapsed + 0.6, random);
    } else if (pattern === 4) {
      ambient.groundPulseStart = elapsed;
    } else if (pattern === 5) {
      spawnComet(effects, elapsed, random);
    } else if (pattern === 6) {
      spawnAperture(effects, elapsed, random);
      spawnNebula(effects, elapsed + 1.1, random);
    } else {
      ambient.mistSurgeStart = elapsed;
    }
    ambient.nextMinorAt = elapsed + minorEffectDelay(random());
  }
  if (elapsed >= ambient.nextMajorAt) {
    ambient.starFlashStart = elapsed;
    const cluster = random();
    if (cluster < 0.3) {
      spawnComet(effects, elapsed, random);
      spawnAperture(effects, elapsed + 1.2, random);
    } else if (cluster < 0.58) {
      ambient.groundPulseStart = elapsed;
      spawnSilhouette(effects, elapsed + 0.8, random);
    } else if (cluster < 0.8) {
      ambient.mistSurgeStart = elapsed;
      spawnNebula(effects, elapsed, random);
      spawnAperture(effects, elapsed + 1.4, random);
    } else {
      spawnLightning(effects, elapsed, random);
      spawnSilhouette(effects, elapsed + 1.1, random);
    }
    ambient.nextMajorAt = elapsed + majorEffectDelay(random());
  }
}

function updateBackgroundEffects(
  effects: BackgroundEffects,
  elapsed: number,
  reducedMotion: boolean,
): void {
  effects.comets.forEach((comet) => {
    const age = (elapsed - comet.start) / comet.duration;
    const visible = !reducedMotion && age >= 0 && age <= 1;
    comet.line.visible = visible;
    comet.head.visible = visible;
    if (!visible) {
      return;
    }
    const envelope = smoothstep(0, 0.12, age) * (1 - smoothstep(0.64, 1, age));
    const position = comet.line.geometry.getAttribute("position") as THREE.BufferAttribute;
    const values = position.array as Float32Array;
    const pointCount = values.length / 3;
    for (let index = 0; index < pointCount; index += 1) {
      const sample = clamp(age - (index / Math.max(pointCount - 1, 1)) * 0.24, 0, 1);
      const offset = index * 3;
      values[offset] = mix(comet.from.x, comet.to.x, sample);
      values[offset + 1] =
        mix(comet.from.y, comet.to.y, sample) + Math.sin(sample * Math.PI) * comet.bend;
      values[offset + 2] = mix(comet.from.z, comet.to.z, sample);
    }
    position.needsUpdate = true;
    const headY = mix(comet.from.y, comet.to.y, age) + Math.sin(age * Math.PI) * comet.bend;
    comet.head.position.set(
      mix(comet.from.x, comet.to.x, age),
      headY,
      mix(comet.from.z, comet.to.z, age),
    );
    comet.line.material.opacity = envelope * 0.82;
    comet.headMaterial.opacity = envelope * 0.76;
  });

  effects.lightning.forEach((lightning) => {
    const age = (elapsed - lightning.start) / lightning.duration;
    const visible = !reducedMotion && age >= 0 && age <= 1;
    lightning.line.visible = visible;
    if (visible) {
      lightning.line.material.opacity = Math.pow(Math.sin(age * Math.PI), 1.4) * 0.58;
    }
  });

  effects.silhouettes.forEach((silhouette) => {
    const age = (elapsed - silhouette.start) / silhouette.duration;
    const visible = !reducedMotion && age >= 0 && age <= 1;
    silhouette.sprite.visible = visible;
    if (!visible) {
      return;
    }
    const envelope = smoothstep(0, 0.18, age) * (1 - smoothstep(0.58, 1, age));
    silhouette.material.opacity = envelope * 0.72;
    silhouette.sprite.position.x =
      silhouette.baseX + Math.sin(elapsed * 0.2 + silhouette.phase) * 11;
    silhouette.sprite.position.y =
      silhouette.baseY + Math.sin(elapsed * 0.13 + silhouette.phase) * 5;
  });

  effects.nebulae.forEach((nebula) => {
    const age = (elapsed - nebula.start) / nebula.duration;
    const visible = !reducedMotion && age >= 0 && age <= 1;
    nebula.sprite.visible = visible;
    if (!visible) {
      return;
    }
    const envelope = smoothstep(0, 0.26, age) * (1 - smoothstep(0.64, 1, age));
    nebula.material.opacity = envelope * 0.28;
    const expansion = 1 + Math.sin(age * Math.PI) * 0.13;
    nebula.sprite.scale.set(
      nebula.baseScale.x * expansion,
      nebula.baseScale.y * expansion,
      1,
    );
    nebula.sprite.position.copy(nebula.base);
    nebula.sprite.position.x += Math.sin(elapsed * 0.035 + nebula.phase) * 0.42;
    nebula.sprite.position.y += Math.sin(elapsed * 0.027 + nebula.phase) * 0.18;
    nebula.material.rotation = Math.sin(elapsed * 0.018 + nebula.phase) * 0.08;
  });

  effects.apertures.forEach((aperture) => {
    const age = (elapsed - aperture.start) / aperture.duration;
    const visible = !reducedMotion && age >= 0 && age <= 1;
    aperture.sprite.visible = visible;
    if (!visible) {
      return;
    }
    const envelope = smoothstep(0, 0.24, age) * (1 - smoothstep(0.58, 1, age));
    aperture.material.opacity = envelope * 0.32;
    const breathing = 1 + Math.sin(age * Math.PI) * 0.12;
    aperture.sprite.scale.setScalar((1.42 + aperture.phase * 0.04) * breathing);
    aperture.sprite.position.copy(aperture.base);
    aperture.sprite.position.y += Math.sin(elapsed * 0.09 + aperture.phase) * 0.22;
    aperture.material.rotation = elapsed * 0.018 * (aperture.phase > Math.PI ? -1 : 1);
  });
}

function spawnComet(
  effects: BackgroundEffects,
  start: number,
  random: () => number,
): void {
  const comet = [...effects.comets].sort(
    (left, right) => left.start + left.duration - (right.start + right.duration),
  )[0];
  if (!comet) {
    return;
  }
  const side = random() > 0.5 ? 1 : -1;
  comet.start = start;
  comet.duration = 2.1 + random() * 1.5;
  comet.from.set(side * (7.2 + random() * 3.4), 2.7 + random() * 3.1, -10 - random() * 8);
  comet.to.set(side * (0.8 + random() * 3.2), -1.2 + random() * 2.2, -8 - random() * 6);
  comet.bend = (random() - 0.35) * 1.8;
}

function spawnLightning(
  effects: BackgroundEffects,
  start: number,
  random: () => number,
): void {
  const lightning = [...effects.lightning].sort(
    (left, right) => left.start + left.duration - (right.start + right.duration),
  )[0];
  if (!lightning) {
    return;
  }
  lightning.start = start;
  lightning.duration = 0.68 + random() * 0.58;
  const side = random() > 0.5 ? 1 : -1;
  const position = lightning.line.geometry.getAttribute("position") as THREE.BufferAttribute;
  const values = position.array as Float32Array;
  const pointCount = values.length / 3;
  const originX = side * (3.5 + random() * 4.2);
  let x = originX;
  for (let index = 0; index < pointCount; index += 1) {
    const progress = index / Math.max(pointCount - 1, 1);
    x += (random() - 0.5) * (index % 3 === 0 ? 0.9 : 0.42);
    values[index * 3] = x;
    values[index * 3 + 1] = 5.2 - progress * (5.5 + random() * 1.4);
    values[index * 3 + 2] = -9.5 - random() * 5.5;
  }
  position.needsUpdate = true;
}

function spawnSilhouette(
  effects: BackgroundEffects,
  start: number,
  random: () => number,
): void {
  const silhouette = [...effects.silhouettes].sort(
    (left, right) => left.start + left.duration - (right.start + right.duration),
  )[0];
  if (!silhouette) {
    return;
  }
  silhouette.start = start;
  silhouette.duration = 4.8 + random() * 3.2;
}

function spawnNebula(
  effects: BackgroundEffects,
  start: number,
  random: () => number,
): void {
  const nebula = [...effects.nebulae].sort(
    (left, right) => left.start + left.duration - (right.start + right.duration),
  )[0];
  if (!nebula) {
    return;
  }
  const side = random() > 0.5 ? 1 : -1;
  nebula.start = start;
  nebula.duration = 10 + random() * 6;
  nebula.phase = random() * Math.PI * 2;
  nebula.base.set(
    side * (3.8 + random() * 3.3),
    0.3 + random() * 3.2,
    -13 - random() * 8,
  );
}

function spawnAperture(
  effects: BackgroundEffects,
  start: number,
  random: () => number,
): void {
  const aperture = [...effects.apertures].sort(
    (left, right) => left.start + left.duration - (right.start + right.duration),
  )[0];
  if (!aperture) {
    return;
  }
  const side = random() > 0.5 ? 1 : -1;
  aperture.start = start;
  aperture.duration = 7.5 + random() * 4.5;
  aperture.phase = random() * Math.PI * 2;
  aperture.base.set(
    side * (4.6 + random() * 3.2),
    0.4 + random() * 3.2,
    -13 - random() * 8,
  );
}

function addLockBox(
  parent: THREE.Group,
  size: readonly [number, number, number],
  position: readonly [number, number, number],
  material: THREE.MeshStandardMaterial,
): THREE.Mesh<THREE.BoxGeometry, THREE.MeshStandardMaterial> {
  const geometry = new THREE.BoxGeometry(...size);
  const mesh = new THREE.Mesh(geometry, material);
  mesh.position.set(...position);
  parent.add(mesh);
  const edges = new THREE.LineSegments(
    new THREE.EdgesGeometry(geometry),
    new THREE.LineBasicMaterial({
      color: COLORS.dim,
      transparent: true,
      opacity: 0.62,
    }),
  );
  edges.position.copy(mesh.position);
  parent.add(edges);
  return mesh;
}

function addCabinetBox(
  parent: THREE.Group,
  shelfId: string,
  size: readonly [number, number, number],
  position: readonly [number, number, number],
  material: THREE.MeshStandardMaterial,
  pickables: THREE.Object3D[],
): THREE.Mesh<THREE.BoxGeometry, THREE.MeshStandardMaterial> {
  const geometry = new THREE.BoxGeometry(...size);
  const mesh = new THREE.Mesh(geometry, material);
  mesh.position.set(...position);
  mesh.userData.shelfId = shelfId;
  parent.add(mesh);
  pickables.push(mesh);
  const edges = new THREE.LineSegments(
    new THREE.EdgesGeometry(geometry),
    new THREE.LineBasicMaterial({ color: 0xb8b1ba, transparent: true, opacity: 0.68 }),
  );
  edges.position.copy(mesh.position);
  parent.add(edges);
  return mesh;
}

function labelPlane(
  text: string,
  width: number,
  height: number,
  fontSize: number,
  foreground: number,
  background: number,
): THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial> {
  const canvas = document.createElement("canvas");
  canvas.width = 512;
  canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for shelf labels");
  }
  context.imageSmoothingEnabled = false;
  context.fillStyle = cssColor(background);
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.strokeStyle = cssColor(foreground);
  context.lineWidth = 5;
  context.strokeRect(5, 5, canvas.width - 10, canvas.height - 10);
  context.fillStyle = cssColor(foreground);
  context.textAlign = "center";
  context.textBaseline = "middle";
  const label = text.toUpperCase();
  const fittedFontSize = fitLabelFontSize(
    context,
    label,
    fontSize,
    canvas.width - 34,
  );
  context.font = `${fittedFontSize}px "Departure Mono", monospace`;
  context.fillText(label, canvas.width / 2, canvas.height / 2 + 3);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  const material = new THREE.MeshBasicMaterial({
    map: texture,
    transparent: true,
    opacity: 1,
    side: THREE.DoubleSide,
    toneMapped: false,
  });
  return new THREE.Mesh(new THREE.PlaneGeometry(width, height), material);
}

function radialGlowTexture(): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for glow textures");
  }
  const gradient = context.createRadialGradient(64, 64, 5, 64, 64, 63);
  gradient.addColorStop(0, "rgba(255,255,255,0.95)");
  gradient.addColorStop(0.28, "rgba(210,140,255,0.62)");
  gradient.addColorStop(0.62, "rgba(120,55,160,0.25)");
  gradient.addColorStop(1, "rgba(0,0,0,0)");
  context.fillStyle = gradient;
  context.fillRect(0, 0, 128, 128);
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function perimeterGlowTexture(): THREE.CanvasTexture {
  const width = 256;
  const height = 384;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for perimeter glow textures");
  }
  context.clearRect(0, 0, width, height);
  for (let layer = 18; layer >= 0; layer -= 1) {
    const position = layer / 18;
    const inset = 25 - layer * 0.72;
    context.strokeStyle = `rgba(181,104,224,${0.012 + position * 0.022})`;
    context.lineWidth = 2 + layer * 1.25;
    context.strokeRect(
      inset,
      inset + 13,
      width - inset * 2,
      height - inset * 2 - 26,
    );
  }
  context.strokeStyle = "rgba(216,160,255,0.62)";
  context.lineWidth = 3;
  context.strokeRect(25, 38, width - 50, height - 76);
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearFilter;
  texture.generateMipmaps = false;
  return texture;
}

function groundSigilTexture(): THREE.CanvasTexture {
  const size = 1024;
  const center = size / 2;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for the ground sigil");
  }
  context.imageSmoothingEnabled = false;
  context.clearRect(0, 0, size, size);
  context.strokeStyle = "rgba(232,227,216,0.82)";
  context.fillStyle = "rgba(216,160,255,0.76)";
  context.lineWidth = 3;

  for (const radius of [445, 421, 382, 330, 248, 188, 102]) {
    context.beginPath();
    context.arc(center, center, radius, 0, Math.PI * 2);
    context.stroke();
  }
  polygonPath(context, center, center, 382, 12, -Math.PI / 2);
  context.stroke();
  polygonPath(context, center, center, 330, 8, Math.PI / 8);
  context.stroke();
  starPath(context, center, center, 298, 158, 8, -Math.PI / 2);
  context.stroke();
  starPath(context, center, center, 178, 72, 12, -Math.PI / 2);
  context.stroke();

  for (let index = 0; index < 48; index += 1) {
    const angle = (index / 48) * Math.PI * 2;
    const inner = index % 4 === 0 ? 394 : 404;
    const outer = index % 6 === 0 ? 442 : 430;
    line2d(
      context,
      center + Math.cos(angle) * inner,
      center + Math.sin(angle) * inner,
      center + Math.cos(angle) * outer,
      center + Math.sin(angle) * outer,
    );
    if (index % 3 === 0) {
      diamond2d(
        context,
        center + Math.cos(angle) * 466,
        center + Math.sin(angle) * 466,
        index % 6 === 0 ? 11 : 7,
      );
    }
  }

  for (let orbit = 0; orbit < 5; orbit += 1) {
    context.save();
    context.translate(center, center);
    context.rotate(orbit * 0.37);
    context.scale(1, 0.42 + orbit * 0.05);
    context.beginPath();
    context.arc(0, 0, 220 + orbit * 24, orbit * 0.28, Math.PI * (1.35 + orbit * 0.12));
    context.stroke();
    context.restore();
  }

  for (const [x, y, radius] of [
    [center, center, 34],
    [center - 228, center + 35, 23],
    [center + 240, center - 74, 19],
    [center - 92, center - 256, 17],
    [center + 114, center + 242, 21],
  ] as const) {
    context.beginPath();
    context.arc(x, y, radius, 0, Math.PI * 2);
    context.stroke();
    diamond2d(context, x, y, radius * 0.45);
  }

  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function groundEchoTexture(layer: number): THREE.CanvasTexture {
  const size = 512;
  const center = size / 2;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for ground echo textures");
  }
  context.clearRect(0, 0, size, size);
  context.strokeStyle = layer === 0
    ? "rgba(216,160,255,0.72)"
    : "rgba(232,227,216,0.58)";
  context.lineWidth = 2;
  for (const radius of layer === 0 ? [188, 206, 224] : [174, 232]) {
    context.beginPath();
    context.arc(center, center, radius, 0, Math.PI * 2);
    context.stroke();
  }
  const markers = layer === 0 ? 16 : 12;
  for (let index = 0; index < markers; index += 1) {
    const angle = index / markers * Math.PI * 2 + layer * Math.PI / 12;
    const radius = layer === 0 ? 224 : 232;
    diamond2d(
      context,
      center + Math.cos(angle) * radius,
      center + Math.sin(angle) * radius,
      index % 4 === 0 ? 5 : 3,
    );
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function cornerFogTexture(side: -1 | 1, seed: number): THREE.CanvasTexture {
  const width = 420;
  const height = 192;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for fog textures");
  }
  context.imageSmoothingEnabled = false;
  const random = deterministicRandom(0x7f4a + seed * 0x11);
  for (let y = 0; y < height; y += 3) {
    for (let x = 0; x < width; x += 3) {
      const edge = side < 0 ? 1 - x / width : x / width;
      const lower = y / height;
      const wave = 0.72 + Math.sin(x * 0.035 + seed * 2 + y * 0.018) * 0.22;
      const density = Math.pow(Math.max(edge, 0), 1.82) *
        Math.pow(Math.max(lower, 0), 0.38) * wave;
      if (density > 0.028 && random() < Math.min(0.985, density * 1.38)) {
        const alpha = Math.round(Math.min(0.98, density * (0.72 + random() * 0.48)) * 255);
        context.fillStyle = `rgba(220,216,224,${alpha / 255})`;
        const block = random() > 0.78 ? 6 : 3;
        if (random() > 0.67) {
          context.beginPath();
          context.arc(x + block / 2, y + block / 2, block * 0.52, 0, Math.PI * 2);
          context.fill();
        } else {
          context.fillRect(x, y, block, block);
        }
      }
    }
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function nebulaTexture(seed: number): THREE.CanvasTexture {
  const source = document.createElement("canvas");
  source.width = 96;
  source.height = 48;
  const sourceContext = source.getContext("2d");
  if (!sourceContext) {
    throw new Error("2D canvas is unavailable for nebula textures");
  }
  const random = deterministicRandom(0x9e17 + seed * 0x51);
  sourceContext.globalCompositeOperation = "lighter";
  for (let index = 0; index < 24; index += 1) {
    const x = 10 + random() * 76;
    const y = 8 + random() * 32;
    const radius = 5 + random() * 15;
    const gradient = sourceContext.createRadialGradient(x, y, 0, x, y, radius);
    const pale = index % 7 === 0;
    gradient.addColorStop(0, pale ? "rgba(220,190,242,0.18)" : "rgba(128,67,157,0.2)");
    gradient.addColorStop(0.42, "rgba(92,42,122,0.12)");
    gradient.addColorStop(1, "rgba(20,8,30,0)");
    sourceContext.fillStyle = gradient;
    sourceContext.fillRect(x - radius, y - radius, radius * 2, radius * 2);
  }
  sourceContext.fillStyle = "rgba(232,227,216,0.42)";
  for (let index = 0; index < 18; index += 1) {
    const size = random() > 0.82 ? 2 : 1;
    sourceContext.fillRect(Math.floor(random() * 96), Math.floor(random() * 48), size, size);
  }

  const canvas = document.createElement("canvas");
  canvas.width = 384;
  canvas.height = 192;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for nebula textures");
  }
  context.imageSmoothingEnabled = false;
  context.drawImage(source, 0, 0, canvas.width, canvas.height);
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function mysteriousSilhouetteTexture(kind: number): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 192;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for silhouette textures");
  }
  context.imageSmoothingEnabled = false;
  context.fillStyle = "rgba(2,2,4,0.94)";
  context.strokeStyle = "rgba(184,118,224,0.62)";
  context.lineWidth = 3;
  context.lineCap = "square";
  context.lineJoin = "miter";

  if (kind % 4 === 0) {
    context.beginPath();
    context.moveTo(22, 186);
    context.quadraticCurveTo(32, 104, 48, 76);
    context.lineTo(42, 52);
    context.quadraticCurveTo(64, 22, 86, 52);
    context.lineTo(80, 78);
    context.quadraticCurveTo(104, 112, 110, 186);
    context.closePath();
    context.fill();
    context.stroke();
    context.fillStyle = "rgba(210,150,244,0.72)";
    context.fillRect(55, 61, 5, 3);
    context.fillRect(69, 61, 5, 3);
  } else if (kind % 4 === 1) {
    context.beginPath();
    context.ellipse(64, 74, 18, 25, 0, 0, Math.PI * 2);
    context.fill();
    context.stroke();
    context.beginPath();
    context.moveTo(49, 56);
    context.lineTo(30, 32);
    context.lineTo(24, 10);
    context.moveTo(36, 39);
    context.lineTo(16, 28);
    context.moveTo(79, 56);
    context.lineTo(98, 32);
    context.lineTo(104, 10);
    context.moveTo(92, 39);
    context.lineTo(112, 28);
    context.stroke();
    context.beginPath();
    context.moveTo(16, 188);
    context.quadraticCurveTo(26, 104, 64, 92);
    context.quadraticCurveTo(102, 104, 112, 188);
    context.closePath();
    context.fill();
    context.stroke();
  } else if (kind % 4 === 2) {
    context.beginPath();
    context.moveTo(8, 186);
    context.quadraticCurveTo(26, 118, 51, 91);
    context.quadraticCurveTo(60, 78, 64, 48);
    context.quadraticCurveTo(68, 78, 77, 91);
    context.quadraticCurveTo(102, 118, 120, 186);
    context.closePath();
    context.fill();
    context.stroke();
    for (const side of [-1, 1]) {
      context.beginPath();
      context.moveTo(64 + side * 11, 98);
      context.quadraticCurveTo(64 + side * 42, 70, 64 + side * 55, 38);
      context.quadraticCurveTo(64 + side * 31, 54, 64 + side * 18, 72);
      context.stroke();
    }
  } else {
    context.fillRect(34, 54, 60, 134);
    context.strokeRect(34, 54, 60, 134);
    context.beginPath();
    context.moveTo(26, 56);
    context.lineTo(64, 12);
    context.lineTo(102, 56);
    context.moveTo(48, 188);
    context.lineTo(48, 92);
    context.quadraticCurveTo(64, 68, 80, 92);
    context.lineTo(80, 188);
    context.stroke();
  }

  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function arcaneApertureTexture(kind: number): THREE.CanvasTexture {
  const size = 192;
  const center = size / 2;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2D canvas is unavailable for aperture textures");
  }
  context.clearRect(0, 0, size, size);
  context.strokeStyle = kind % 2 === 0
    ? "rgba(198,126,236,0.76)"
    : "rgba(232,227,216,0.68)";
  context.lineWidth = 2;
  context.setLineDash([4 + kind, 5 + kind * 2]);
  for (const radius of [66, 52, 34]) {
    context.beginPath();
    context.arc(center, center, radius, kind * 0.42, Math.PI * (1.6 + kind * 0.08));
    context.stroke();
  }
  context.setLineDash([]);
  polygonPath(context, center, center, 58, 6 + kind * 2, -Math.PI / 2);
  context.stroke();
  diamond2d(context, center, center, 12 + kind * 2);
  for (let index = 0; index < 8; index += 1) {
    const angle = index / 8 * Math.PI * 2 + kind * 0.21;
    diamond2d(
      context,
      center + Math.cos(angle) * 76,
      center + Math.sin(angle) * 76,
      index % 2 === 0 ? 4 : 2,
    );
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return texture;
}

function circleLine(
  radius: number,
  color: number,
  opacity: number,
  segments: number,
): THREE.LineLoop<THREE.BufferGeometry, THREE.LineBasicMaterial> {
  const points = Array.from({ length: segments }, (_, index) => {
    const angle = (index / segments) * Math.PI * 2;
    return new THREE.Vector3(Math.cos(angle) * radius, Math.sin(angle) * radius, 0);
  });
  const geometry = new THREE.BufferGeometry().setFromPoints(points);
  const material = new THREE.LineBasicMaterial({ color, transparent: true, opacity });
  return new THREE.LineLoop(geometry, material);
}

function starLine(
  outer: number,
  inner: number,
  points: number,
  color: number,
  opacity: number,
): THREE.LineLoop<THREE.BufferGeometry, THREE.LineBasicMaterial> {
  const vertices = Array.from({ length: points * 2 }, (_, index) => {
    const radius = index % 2 === 0 ? outer : inner;
    const angle = -Math.PI / 2 + (index / (points * 2)) * Math.PI * 2;
    return new THREE.Vector3(Math.cos(angle) * radius, Math.sin(angle) * radius, 0);
  });
  return new THREE.LineLoop(
    new THREE.BufferGeometry().setFromPoints(vertices),
    new THREE.LineBasicMaterial({ color, transparent: true, opacity }),
  );
}

function lineSegments(
  values: number[],
  color: number,
  opacity: number,
): THREE.LineSegments<THREE.BufferGeometry, THREE.LineBasicMaterial> {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(values, 3));
  return new THREE.LineSegments(
    geometry,
    new THREE.LineBasicMaterial({ color, transparent: true, opacity }),
  );
}

function projectWithCamera(
  id: string,
  position: THREE.Vector3,
  camera: THREE.PerspectiveCamera,
): ProjectedShelf {
  const projected = position.clone().project(camera);
  return {
    id,
    x: (projected.x * 0.5 + 0.5) * SCENE_WIDTH,
    y: (-projected.y * 0.5 + 0.5) * SCENE_HEIGHT,
    scale: clamp(1 - (position.z + 5) * 0.06, 0.45, 1),
    alpha: 1,
  };
}

function projectWorldPosition(id: string, position: THREE.Vector3): ProjectedShelf {
  return {
    id,
    x: SCENE_WIDTH / 2 + position.x * 95,
    y: SCENE_HEIGHT / 2 - position.y * 95,
    scale: 0.75,
    alpha: 1,
  };
}

function motionToWorld(motion: ShelfMotion): THREE.Vector3 {
  return new THREE.Vector3(
    motion.x * 5.45,
    motion.y * 2.62 + 0.05,
    -1.65 - motion.z * 4,
  );
}

function worldToMotion(position: THREE.Vector3): ShelfMotion {
  return {
    x: clamp(position.x / 5.45, -DRAG_LIMIT_X, DRAG_LIMIT_X),
    y: clamp((position.y - 0.05) / 2.62, -DRAG_LIMIT_Y, DRAG_LIMIT_Y),
    z: clamp((-1.65 - position.z) / 4, SHELF_MOTION_BOUNDS.minimumZ, SHELF_MOTION_BOUNDS.maximumZ),
    velocityX: 0,
    velocityY: 0,
    velocityZ: 0,
  };
}

function clampMotionToCell(
  motion: ShelfMotion,
  cell: ShelfRoamingCell,
): ShelfMotion {
  return {
    ...motion,
    x: clamp(motion.x, cell.minimumX, cell.maximumX),
    y: clamp(motion.y, cell.minimumY, cell.maximumY),
    z: clamp(motion.z, cell.minimumZ, cell.maximumZ),
  };
}

function romanFromVolume(volume: LibraryVolume): string {
  return volume.label.replace(/^VOL\s*/i, "") || String(volume.index);
}

function fitLabelFontSize(
  context: CanvasRenderingContext2D,
  value: string,
  requestedSize: number,
  maxWidth: number,
): number {
  let size = requestedSize;
  context.font = `${size}px "Departure Mono", monospace`;
  while (size > 22 && context.measureText(value).width > maxWidth) {
    size -= 1;
    context.font = `${size}px "Departure Mono", monospace`;
  }
  return size;
}

function polygonPath(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  points: number,
  rotation: number,
): void {
  context.beginPath();
  for (let index = 0; index <= points; index += 1) {
    const angle = rotation + (index / points) * Math.PI * 2;
    const px = x + Math.cos(angle) * radius;
    const py = y + Math.sin(angle) * radius;
    if (index === 0) {
      context.moveTo(px, py);
    } else {
      context.lineTo(px, py);
    }
  }
  context.closePath();
}

function starPath(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  outer: number,
  inner: number,
  points: number,
  rotation: number,
): void {
  context.beginPath();
  for (let index = 0; index <= points * 2; index += 1) {
    const radius = index % 2 === 0 ? outer : inner;
    const angle = rotation + (index / (points * 2)) * Math.PI * 2;
    const px = x + Math.cos(angle) * radius;
    const py = y + Math.sin(angle) * radius;
    if (index === 0) {
      context.moveTo(px, py);
    } else {
      context.lineTo(px, py);
    }
  }
  context.closePath();
}

function line2d(
  context: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
): void {
  context.beginPath();
  context.moveTo(x1, y1);
  context.lineTo(x2, y2);
  context.stroke();
}

function diamond2d(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
): void {
  context.beginPath();
  context.moveTo(x, y - radius);
  context.lineTo(x + radius, y);
  context.lineTo(x, y + radius);
  context.lineTo(x - radius, y);
  context.closePath();
  context.stroke();
}

function setOpacity(material: THREE.Material, opacity: number): void {
  if ("opacity" in material && typeof material.opacity === "number") {
    material.opacity = opacity;
    material.transparent = opacity < 0.999;
  }
}

function disposeScene(scene: THREE.Scene): void {
  const textures = new Set<THREE.Texture>();
  scene.traverse((object) => {
    if (object instanceof THREE.Mesh || object instanceof THREE.Line || object instanceof THREE.Points) {
      object.geometry?.dispose();
      const materials = Array.isArray(object.material) ? object.material : [object.material];
      for (const material of materials) {
        for (const value of Object.values(material)) {
          if (value instanceof THREE.Texture) {
            textures.add(value);
          }
        }
        material.dispose();
      }
    }
    if (object instanceof THREE.Sprite) {
      if (object.material.map) {
        textures.add(object.material.map);
      }
      object.material.dispose();
    }
  });
  textures.forEach((texture) => texture.dispose());
  scene.clear();
}

function cssColor(value: number): string {
  return `#${value.toString(16).padStart(6, "0")}`;
}

function deterministicRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
    return state / 0xffff_ffff;
  };
}

function moveTowards(current: number, target: number, amount: number): number {
  if (current < target) {
    return Math.min(current + amount, target);
  }
  return Math.max(current - amount, target);
}

function smoothstep(edge0: number, edge1: number, value: number): number {
  const amount = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return amount * amount * (3 - 2 * amount);
}

function mix(start: number, end: number, amount: number): number {
  return start + (end - start) * amount;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}
