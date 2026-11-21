import type { Point } from "./scene";
import { ACCENT } from "./theme";

/** The geometry the loupe needs from the overlay's capture meta. */
export type MagnifierInfo = {
  pointsWidth: number;
  pointsHeight: number;
  scale: number;
};

/** Reads the frozen frame's colour under a point, in points. */
export function sample(image: ImageData | null, scale: number, point: Point) {
  if (!image) return null;
  const x = Math.min(image.width - 1, Math.max(0, Math.round(point.x * scale)));
  const y = Math.min(image.height - 1, Math.max(0, Math.round(point.y * scale)));
  const index = (y * image.width + x) * 4;
  const [r, g, b] = [image.data[index], image.data[index + 1], image.data[index + 2]];
  return {
    r,
    g,
    b,
    hex: `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("").toUpperCase()}`,
  };
}

/** 132×99 point loupe with a position/colour readout, drawn on the frozen frame. */
export function drawMagnifier(
  context: CanvasRenderingContext2D,
  background: HTMLCanvasElement,
  image: ImageData | null,
  info: MagnifierInfo,
  cursor: Point,
  drag: { start: Point; current: Point } | null,
  zoom: number,
) {
  const width = 132;
  const height = 99;
  const sourceWidth = width / zoom;
  const sourceHeight = height / zoom;

  let left = cursor.x + 24;
  let top = cursor.y + 24;
  if (left + width > info.pointsWidth) left = cursor.x - width - 24;
  if (top + height > info.pointsHeight) top = cursor.y - height - 24;
  left = Math.max(4, left);
  top = Math.max(4, top);

  const sourceX = (cursor.x - sourceWidth / 2) * info.scale;
  const sourceY = (cursor.y - sourceHeight / 2) * info.scale;

  context.save();
  context.imageSmoothingEnabled = false;
  context.beginPath();
  context.rect(left, top, width, height);
  context.clip();
  context.fillStyle = "#000";
  context.fillRect(left, top, width, height);
  context.drawImage(
    background,
    sourceX,
    sourceY,
    sourceWidth * info.scale,
    sourceHeight * info.scale,
    left,
    top,
    width,
    height,
  );
  context.restore();

  context.strokeStyle = "rgba(255,255,255,0.35)";
  context.lineWidth = 1;
  context.strokeRect(left + 0.5, top + 0.5, width - 1, height - 1);

  context.strokeStyle = ACCENT;
  context.beginPath();
  const centreX = left + width / 2;
  const centreY = top + height / 2;
  context.moveTo(centreX, centreY - 6);
  context.lineTo(centreX, centreY + 6);
  context.moveTo(centreX - 6, centreY);
  context.lineTo(centreX + 6, centreY);
  context.stroke();

  const colour = sample(image, info.scale, cursor);
  context.fillStyle = "rgba(20,20,22,0.92)";
  context.fillRect(left, top + height, width, 18);
  context.fillStyle = "#f5f5f7";
  context.font = "11px -apple-system, BlinkMacSystemFont, sans-serif";
  context.textBaseline = "middle";
  const size = drag
    ? `  ${Math.abs(Math.round(drag.current.x - drag.start.x))}×${Math.abs(
        Math.round(drag.current.y - drag.start.y),
      )}`
    : "";
  context.fillText(
    `${Math.round(cursor.x)}, ${Math.round(cursor.y)}   ${colour?.hex ?? ""}${size}`,
    left + 6,
    top + height + 9,
  );
  if (colour) {
    context.fillStyle = colour.hex;
    context.fillRect(left + width - 16, top + height + 4, 12, 10);
  }
}
