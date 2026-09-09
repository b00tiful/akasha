import { skyRandom } from "./local-akasha-layout";

// One paint at mount: atmosphere has no DOM population, GPU context, or idle frame loop.
export function paintLocalSky(canvas: HTMLCanvasElement, seed: string): void {
  canvas.width = 1600;
  canvas.height = 900;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const random = skyRandom(seed);
  ctx.fillStyle = "#030304";
  ctx.fillRect(0, 0, 1600, 900);

  // Sparse, broken dust along two oblique bands; no semantic connections to section stars.
  for (let i = 0; i < 12500; i++) {
    const t = random();
    const width = (random() + random() + random() - 1.5) * 135;
    const x = 940 + t * 700 + width;
    const y = 920 - t * 790 + Math.sin(t * 19) * 35 + width * 0.35;
    ctx.fillStyle = `rgba(155,139,171,${random() * 0.055})`;
    ctx.fillRect(Math.round(x), Math.round(y), 1 + random() * 2, 1);
  }
  for (let i = 0; i < 700; i++) {
    const x = Math.floor(random() * 1600);
    const y = Math.floor(random() * 900);
    const bright = random();
    ctx.fillStyle = `rgba(232,227,216,${0.09 + bright ** 5 * 0.68})`;
    ctx.fillRect(x, y, bright > 0.99 ? 2 : 1, bright > 0.99 ? 2 : 1);
    if (bright > 0.994) {
      ctx.globalAlpha = 0.4;
      ctx.fillRect(x - 5, y, 11, 1);
      ctx.fillRect(x, y - 7, 1, 15);
      ctx.globalAlpha = 1;
    }
  }
  ctx.strokeStyle = "rgba(203,195,177,0.14)";
  ctx.lineWidth = 0.7;
  for (const [x, y] of [[100, 170], [1420, 160], [1150, 650]]) {
    ctx.beginPath();
    ctx.moveTo(x!, y!);
    for (let i = 0; i < 4; i++) ctx.lineTo(x! + random() * 100, y! + random() * 140);
    ctx.stroke();
  }

  // Distant etched ridge and observatory: environmental accents confined to a corner.
  for (let layer = 0; layer < 3; layer++) {
    ctx.beginPath();
    ctx.moveTo(0, 740 + layer * 28);
    for (let x = 0; x <= 630; x += 12) {
      ctx.lineTo(x, 738 + x * 0.25 + layer * 20 - random() * (42 - layer * 9));
    }
    ctx.lineTo(630, 900);
    ctx.lineTo(0, 900);
    ctx.fillStyle = "#030304";
    ctx.fill();
    ctx.strokeStyle = `rgba(205,197,180,${0.26 - layer * 0.065})`;
    ctx.stroke();
  }
  ctx.beginPath();
  ctx.moveTo(43, 747); ctx.lineTo(57, 681); ctx.lineTo(65, 710);
  ctx.lineTo(71, 617); ctx.lineTo(78, 701); ctx.lineTo(86, 681); ctx.lineTo(106, 760);
  ctx.strokeStyle = "#746f62";
  ctx.stroke();
  ctx.beginPath(); ctx.arc(71, 592, 17, 0.4, 5.1); ctx.stroke();
  ctx.beginPath(); ctx.arc(78, 586, 17, 1, 4.1); ctx.stroke();
}
