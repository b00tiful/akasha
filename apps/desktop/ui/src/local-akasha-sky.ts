import { LOCAL_TRAVEL_MS, hashSky, skyRandom, type SkyPoint } from "./local-akasha-layout";

export type LocalSkyEvent = "comet" | "rift" | "eclipse";
export interface LocalSkyHandle {
  destroy(): void;
  setReducedMotion(value: boolean): void;
  setPaused(value: boolean): void;
  travel(from: SkyPoint, to: SkyPoint, reverse: boolean): void;
  settle(): void;
}

// One slow event at most, separated by long quiet intervals. No flash/strobe envelopes.
export function localSkyEvent(time: number, seed: string): { kind: LocalSkyEvent; progress: number } | null {
  const offset = hashSky(seed) % 11;
  const cycle = Math.floor((time - 18 - offset) / 37);
  const age = time - 18 - offset - cycle * 37;
  if (cycle < 0 || age > 5) return null;
  return { kind: (["comet", "eclipse", "rift"] as const)[cycle % 3]!, progress: age / 5 };
}

const W = 640;
const H = 360;
const VOID = "#030304";
const INK = "#211c29";
const DIM = "#51495b";
const PAPER = "#d8d1c2";
const VIOLET = "#9474af";

export function mountLocalSky(canvas: HTMLCanvasElement, seed: string, reduced: boolean): LocalSkyHandle {
  canvas.width = W; canvas.height = H;
  const ctx = canvas.getContext("2d");
  if (!ctx) return { destroy() {}, setReducedMotion() {}, setPaused() {}, travel() {}, settle() {} };
  ctx.imageSmoothingEnabled = false;
  const random = skyRandom(seed);
  const stars = Array.from({ length: 460 }, () => ({ x: random() * W, y: random() * H,
    phase: random() * 6.28, speed: .18 + random() * .45, bright: random(), purple: random() < .065 }));
  const motes = Array.from({ length: 46 }, () => ({ x: random() * W, y: random() * H,
    speed: .5 + random() * 1.2, phase: random() * 6.28 }));
  const dust = [0, 1].map((layer) => {
    const texture = document.createElement("canvas"); texture.width = W; texture.height = H;
    const surface = texture.getContext("2d")!;
    const density = new Uint16Array(W * H);
    for (let i = 0; i < 44000; i++) {
      const t = random();
      const scatter = (random() + random() + random() - 1.5) * (layer ? 44 : 62);
      const x = Math.round((layer ? 390 : 120) + t * (layer ? 235 : 420) + scatter);
      const y = Math.round(365 - t * 330 + Math.sin(t * 15 + layer) * 22 + scatter * .6);
      if (x >= 0 && x < W && y >= 0 && y < H) density[y * W + x]!++;
    }
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
      const value = density[y * W + x]!;
      if (value < 2 || random() > .42) continue;
      surface.fillStyle = value > 13 ? (random() < .13 ? VIOLET : DIM) : INK;
      surface.fillRect(x, y, 1, 1);
    }
    return texture;
  });
  const landscape = document.createElement("canvas"); landscape.width = W; landscape.height = H;
  const land = landscape.getContext("2d")!;
  function pixelLine(context: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number, skip = 1): void {
    const steps = Math.max(Math.abs(x2 - x1), Math.abs(y2 - y1), 1);
    for (let i = 0; i <= steps; i += skip) context.fillRect(Math.round(x1 + (x2 - x1) * i / steps), Math.round(y1 + (y2 - y1) * i / steps), 1, 1);
  }
  function ridge(vertices: number[][], color: string): void {
    land.beginPath(); land.moveTo(vertices[0]![0]!, vertices[0]![1]!);
    for (const [x, y] of vertices) land.lineTo(x!, y!);
    land.lineTo(vertices.at(-1)![0]!, H); land.lineTo(0, H); land.closePath();
    land.fillStyle = VOID; land.fill(); land.fillStyle = color;
    vertices.slice(1).forEach(([x, y], i) => pixelLine(land, vertices[i]![0]!, vertices[i]![1]!, x!, y!));
  }
  ridge([[0, 310], [18, 301], [35, 303], [55, 278], [70, 288], [79, 306], [102, 318], [124, 315], [164, 340], [219, 350], [250, 360]], INK);
  // A ruined observatory on one broad cliff, with readable broken arches and masonry.
  ridge([[0, 333], [14, 322], [29, 322], [39, 308], [40, 285], [43, 285], [43, 277],
    [47, 277], [50, 250], [53, 277], [57, 277], [57, 293], [65, 293], [65, 272],
    [70, 272], [70, 266], [74, 268], [78, 272], [78, 302], [85, 302], [85, 292],
    [89, 292], [92, 272], [96, 292], [100, 292], [100, 314], [117, 325],
    [132, 324], [154, 342], [196, 351], [233, 360]], DIM);
  land.fillStyle = INK;
  for (let i = 0; i < 470; i++) {
    const x = Math.floor(random() * 180); const y = Math.floor(323 + random() * 37);
    if (y > 319 + x * .18) land.fillRect(x, y, 1 + (i % 3), 1);
  }
  // Inset arch: only narrow side light survives the black silhouette.
  land.fillStyle = INK;
  pixelLine(land, 64, 318, 64, 302); pixelLine(land, 64, 302, 70, 295);
  pixelLine(land, 70, 295, 76, 302); pixelLine(land, 76, 302, 76, 318);
  // Eroded courses, recessed windows, and a broken flying buttress give the ruin mass.
  for (const [x, y, height] of [[45, 287, 26], [54, 295, 18], [68, 276, 15], [74, 277, 16], [89, 298, 17]]) {
    pixelLine(land, x!, y!, x!, y! + height!, 2);
    for (let row = 0; row < height!; row += 4) pixelLine(land, x!, y! + row, x! + 2, y! + row);
  }
  pixelLine(land, 81, 316, 82, 300); pixelLine(land, 82, 300, 87, 293);
  pixelLine(land, 38, 310, 34, 323); pixelLine(land, 103, 317, 110, 324);
  land.fillStyle = DIM;
  pixelLine(land, 43, 283, 55, 283); pixelLine(land, 86, 296, 98, 296);
  pixelLine(land, 62, 292, 64, 290); pixelLine(land, 78, 301, 80, 300);
  land.fillStyle = VIOLET; land.fillRect(49, 282, 1, 4); land.fillRect(91, 298, 1, 3);
  land.fillStyle = DIM;
  for (let i = 0; i < 50; i++) {
    const a = .6 + i / 50 * 4.4;
    land.fillRect(Math.round(54 + Math.cos(a) * 10), Math.round(233 + Math.sin(a) * 10), 1, 1);
  }
  land.fillStyle = INK;
  for (let i = 0; i < 45; i++) {
    const a = .65 + i / 45 * 4.25;
    land.fillRect(Math.round(58 + Math.cos(a) * 10), Math.round(230 + Math.sin(a) * 10), 1, 1);
  }

  let time = 0;
  let last = 0;
  let frame = 0;
  let paused = false;
  let destroyed = false;
  let voyage: { from: SkyPoint; to: SkyPoint; reverse: boolean; start: number } | null = null;
  const forced = new URLSearchParams(location.search).get("localEvent");
  const forcedEvent = ["comet", "rift", "eclipse"].includes(forced ?? "") ? forced as LocalSkyEvent : null;
  function dot(x: number, y: number, color: string): void {
    ctx!.fillStyle = color; ctx!.fillRect(Math.round(x), Math.round(y), 1, 1);
  }
  function flare(x: number, y: number, radius: number, color: string): void {
    dot(x, y, PAPER);
    for (let i = 1; i <= radius; i++) {
      dot(x, y + i, i < 3 ? color : DIM); dot(x, y - i, i < 3 ? color : DIM);
      if (i < radius * .65) { dot(x + i, y, color); dot(x - i, y, color); }
    }
  }
  function ring(x: number, y: number, r: number, angle: number, color: string, reveal = 1): void {
    for (let i = 0; i < 180 * reveal; i++) {
      if (i % 15 > 10) continue;
      const a = i / 180 * Math.PI * 2 + angle;
      dot(x + Math.cos(a) * r, y + Math.sin(a) * r * .74, color);
    }
  }
  function event(kind: LocalSkyEvent, p: number): void {
    const envelope = Math.sin(Math.PI * p);
    if (kind === "comet") {
      const x = W * (.17 + p * .56); const y = H * (.16 + p * .3);
      for (let i = 0; i < 65 * envelope; i++) dot(x - i, y - i * .31 + Math.sin(i * .13) * .4, i < 5 ? PAPER : i < 19 ? VIOLET : INK);
      flare(x, y, 3 * envelope, PAPER);
    } else if (kind === "eclipse") {
      const x = W * .78; const y = H * .27; const r = 18 + p * 4;
      for (let i = 0; i < 140; i++) {
        const a = i / 140 * 6.28;
        if (Math.sin(a * 7 + p * 4) > envelope * 1.5 - .9) continue;
        dot(x + Math.cos(a) * r, y + Math.sin(a) * r, i % 9 === 0 ? VIOLET : DIM);
      }
      ring(x, y, r * 1.6, p, INK, envelope);
    } else {
      const x = W * .77; const y = H * .24;
      for (let branch = 0; branch < 3; branch++) {
        let px = x; let py = y;
        for (let i = 1; i < 32 * envelope; i++) {
          const nx = x - i * (1.6 + branch * .6) + Math.sin(i * 1.2 + branch) * 2;
          const ny = y + i * (branch - .7) + Math.sin(i * .7) * 3;
          ctx!.fillStyle = branch ? INK : DIM;
          pixelLine(ctx!, px, py, nx, ny); px = nx; py = ny;
          if (i % 5 === 0) dot(px, py, VIOLET);
        }
      }
    }
  }
  function draw(): void {
    ctx!.fillStyle = VOID; ctx!.fillRect(0, 0, W, H);
    dust.forEach((texture, i) => ctx!.drawImage(texture, Math.round(Math.sin(time * .028 + i * 2) * 5), Math.round(Math.sin(time * .018 + i) * 3)));
    for (const star of stars) {
      const light = .5 + .5 * Math.sin(time * star.speed + star.phase);
      const color = star.bright > .94 ? (star.purple ? VIOLET : PAPER) : light > .72 ? DIM : INK;
      dot(star.x, star.y, color);
      if (star.bright > .993) flare(star.x, star.y, 2 + Math.floor(light * 3), color);
    }
    for (const mote of motes) {
      const y = (mote.y - time * mote.speed % H + H) % H;
      if (y > H * .6 && Math.sin(time * .3 + mote.phase) > .15) dot(mote.x + Math.sin(time * .08 + mote.phase) * 9, y, DIM);
    }
    ctx!.drawImage(landscape, 0, 0);
    if (!reduced && !paused && !voyage) {
      const current = forcedEvent ? { kind: forcedEvent, progress: (time % 9) / 5 } : localSkyEvent(time, seed);
      if (current && current.progress <= 1) event(current.kind, current.progress);
    }
    if (voyage) {
      const p = Math.min(1, (time - voyage.start) / (LOCAL_TRAVEL_MS / 1000));
      const t = p * p * (3 - 2 * p);
      const x = (voyage.from.x + (voyage.to.x - voyage.from.x) * t) * W;
      const y = (voyage.from.y + (voyage.to.y - voyage.from.y) * t) * H;
      const power = Math.sin(Math.PI * p);
      ring(x, y, 12 + power * 60, p * 1.7, VIOLET, Math.min(1, p * 5));
      ring(x, y, 18 + power * 77, -p * 2, DIM, 1 - p * .6);
      for (let i = 0; i < 74; i++) {
        const a = i * 2.39996 + p * (voyage.reverse ? -4 : 4);
        const radius = (1 - t) * (34 + i * 1.4) + t * 8;
        const px = x + Math.cos(a) * radius; const py = y + Math.sin(a) * radius * .7;
        ctx!.fillStyle = i % 9 === 0 ? VIOLET : DIM;
        pixelLine(ctx!, px, py, px + Math.cos(a) * power * 17, py + Math.sin(a) * power * 11, 2);
      }
      // Eight broken radial seals draw, turn, and disperse around the travelling star.
      for (let i = 0; i < 8; i++) {
        const a = i * Math.PI / 4 - p * .8;
        const r = 23 + power * 27;
        ctx!.fillStyle = i % 3 ? DIM : VIOLET;
        const px = x + Math.cos(a) * r; const py = y + Math.sin(a) * r * .74;
        pixelLine(ctx!, px - 2, py, px, py - 4); pixelLine(ctx!, px, py - 4, px + 3, py + 2);
      }
      flare(x, y, 5 + power * 15, VIOLET);
      if (p >= 1) voyage = null;
    }
  }
  function tick(now: number): void {
    if (destroyed || reduced || paused || document.hidden) { frame = 0; return; }
    if (!last) last = now;
    if (now - last >= 1000 / 30) { time += Math.min(.1, (now - last) / 1000); last = now; draw(); }
    frame = requestAnimationFrame(tick);
  }
  function sync(): void {
    cancelAnimationFrame(frame); frame = 0; last = 0;
    if (!destroyed && !reduced && !paused && !document.hidden) frame = requestAnimationFrame(tick);
  }
  document.addEventListener("visibilitychange", sync);
  draw(); sync();
  return {
    destroy() { destroyed = true; cancelAnimationFrame(frame); document.removeEventListener("visibilitychange", sync); },
    setReducedMotion(value) { reduced = value; if (value) voyage = null; draw(); sync(); },
    setPaused(value) { paused = value; if (value) voyage = null; sync(); },
    travel(from, to, reverse) { if (!reduced) voyage = { from, to, reverse, start: time }; },
    settle() { voyage = null; },
  };
}
