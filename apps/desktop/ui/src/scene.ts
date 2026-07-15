import { Application, Container, Graphics } from "pixi.js";

import { layoutBooks, visibleLinks } from "./projection";
import type { LibraryProjection } from "./types";

const COLORS = {
  void: 0x090b0d,
  ink: 0x11161a,
  paper: 0xe7e0cf,
  global: 0x64d8cb,
  selected: 0xe3a64b,
  link: 0x64d8cb,
  danger: 0xdb5a52,
} as const;
const SCENE_WIDTH = 720;
const SCENE_HEIGHT = 360;

export interface SceneHandle {
  select(id: string): void;
  destroy(): void;
}

export async function mountLibraryScene(
  host: HTMLElement,
  projection: LibraryProjection,
  reducedMotion: boolean,
  onSelect: (id: string) => void,
): Promise<SceneHandle> {
  const app = new Application();
  await app.init({
    width: Math.max(1, host.clientWidth),
    height: Math.max(1, host.clientHeight),
    backgroundColor: COLORS.void,
    antialias: false,
    preference: "webgl",
    resolution: 1,
  });
  app.canvas.className = "world-canvas";
  app.canvas.setAttribute("aria-hidden", "true");
  host.replaceChildren(app.canvas);

  const world = new Container();
  app.stage.addChild(world);
  const resize = (): void => {
    const width = Math.max(1, host.clientWidth);
    const height = Math.max(1, host.clientHeight);
    app.renderer.resize(width, height);
    const scale = Math.min(width / SCENE_WIDTH, height / SCENE_HEIGHT);
    world.scale.set(scale);
    world.position.set(
      Math.floor((width - SCENE_WIDTH * scale) / 2),
      Math.floor((height - SCENE_HEIGHT * scale) / 2),
    );
  };
  const resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(host);
  resize();
  const positioned = layoutBooks(projection);
  const positions = new Map(positioned.map((item) => [item.book.id, item]));
  const bookShapes = new Map<string, Graphics>();

  drawShelves(world, projection);
  const traffic = visibleLinks(projection);
  for (const link of traffic) {
    const source = positions.get(link.source);
    const target = positions.get(link.target);
    if (!source || !target) {
      continue;
    }
    world.addChild(
      new Graphics()
        .moveTo(source.x + 6, source.y + 12)
        .lineTo(target.x + 6, target.y + 12)
        .stroke({ color: COLORS.link, width: 1, alpha: 0.72 }),
    );
  }

  for (const item of positioned) {
    const isGlobal = item.book.scope.kind === "global";
    const shape = new Graphics()
      .rect(item.x, item.y, 12, 24)
      .fill(isGlobal ? COLORS.global : COLORS.paper)
      .stroke({ color: COLORS.void, width: 2 });
    shape.eventMode = "static";
    shape.cursor = "pointer";
    shape.on("pointertap", () => onSelect(item.book.id));
    world.addChild(shape);
    bookShapes.set(item.book.id, shape);
  }

  const route = traffic[0];
  const source = route ? positions.get(route.source) : undefined;
  const target = route ? positions.get(route.target) : undefined;
  const vehicle = new Graphics().rect(-2, -2, 5, 5).fill(COLORS.selected);
  if (source && target) {
    world.addChild(vehicle);
    let progress = reducedMotion ? 0.5 : 0;
    const updateVehicle = (): void => {
      vehicle.x = source.x + 6 + (target.x - source.x) * progress;
      vehicle.y = source.y + 12 + (target.y - source.y) * progress;
    };
    updateVehicle();
    if (!reducedMotion) {
      app.ticker.add((ticker) => {
        progress = (progress + ticker.deltaMS / 4200) % 1;
        updateVehicle();
      });
    }
  }

  return {
    select(id: string): void {
      for (const [bookId, shape] of bookShapes) {
        shape.alpha = bookId === id ? 1 : 0.76;
        shape.tint = bookId === id ? COLORS.selected : 0xffffff;
      }
    },
    destroy(): void {
      resizeObserver.disconnect();
      app.destroy(true);
    },
  };
}

function drawShelves(world: Container, projection: LibraryProjection): void {
  world.addChild(
    new Graphics()
      .roundRect(42, 44, 132, 270, 8)
      .fill(COLORS.ink)
      .stroke({ color: COLORS.global, width: 1 }),
  );
  projection.projects.forEach((_, index) => {
    world.addChild(
      new Graphics()
        .roundRect(204 + index * 172, 44, 146, 270, 8)
        .fill(COLORS.ink)
        .stroke({ color: COLORS.paper, width: 1, alpha: 0.55 }),
    );
  });

  const horizon = new Graphics().rect(24, 330, 672, 2).fill(COLORS.danger);
  horizon.alpha = 0.35;
  world.addChild(horizon);
}
