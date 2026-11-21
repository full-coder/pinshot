/**
 * Canvas rendering of the annotation scene.
 *
 * One renderer only: the overlay uses it for the live preview and the export uses it
 * for the final pixels, so what you see is exactly what gets copied. Text is drawn by
 * the browser, which is also why CJK text is correct without shipping a font
 * rasteriser.
 */

import type { Point, Rect, Shape } from "./scene";

export type RenderOptions = {
  /** Device pixels per point. */
  scale: number;
};

export function drawScene(
  context: CanvasRenderingContext2D,
  shapes: readonly Shape[],
  options: RenderOptions,
): void {
  for (const shape of shapes) drawShape(context, shape, options);
}

export function drawShape(
  context: CanvasRenderingContext2D,
  shape: Shape,
  options: RenderOptions,
): void {
  const { scale } = options;
  context.save();
  context.lineCap = "round";
  context.lineJoin = "round";

  switch (shape.kind) {
    case "rect":
    case "ellipse": {
      context.strokeStyle = shape.style.color;
      context.lineWidth = shape.style.width;
      context.beginPath();
      if (shape.kind === "rect") {
        context.rect(shape.rect.x, shape.rect.y, shape.rect.w, shape.rect.h);
      } else {
        context.ellipse(
          shape.rect.x + shape.rect.w / 2,
          shape.rect.y + shape.rect.h / 2,
          Math.max(0.5, shape.rect.w / 2),
          Math.max(0.5, shape.rect.h / 2),
          0,
          0,
          Math.PI * 2,
        );
      }
      context.stroke();
      break;
    }

    case "line":
    case "pen":
    case "marker": {
      if (shape.points.length < 2) {
        context.restore();
        return;
      }
      context.globalAlpha = shape.kind === "marker" ? (shape.style.alpha ?? 0.35) : 1;
      context.strokeStyle = shape.style.color;
      context.lineWidth =
        shape.kind === "marker" ? shape.style.width * 3 : shape.style.width;
      // An arrow's shaft stops inside its head. Stroking all the way to the tip left the
      // round cap poking out of the triangle, which looked like a bump on the tip.
      const head =
        shape.kind === "line" && shape.arrow
          ? arrowHead(shape.points, shape.style.width)
          : null;
      const shaft = head ? [...shape.points.slice(0, -1), head.shaftEnd] : shape.points;
      strokePath(context, shaft, shape.kind === "line");
      context.globalAlpha = 1;
      if (head) {
        context.fillStyle = shape.style.color;
        fillArrowHead(context, head);
      }
      break;
    }

    case "mosaic": {
      pixelate(context, shape.rect, scale, Math.max(2, shape.block));
      break;
    }

    case "text": {
      const font = `${shape.size}px -apple-system, BlinkMacSystemFont, "PingFang SC", sans-serif`;
      context.font = font;
      context.textBaseline = "alphabetic";
      // No outline: the text has to look exactly like what was typed. Snipaste's default
      // is unstroked too; contrast is the user's choice of colour.
      context.fillStyle = shape.style.color;
      context.fillText(shape.text, shape.at.x, shape.at.y);
      break;
    }

    case "counter": {
      const radius = 14;
      context.beginPath();
      context.arc(shape.at.x, shape.at.y, radius, 0, Math.PI * 2);
      context.fillStyle = shape.style.color;
      context.fill();
      context.fillStyle = "#ffffff";
      context.font = "600 15px -apple-system, BlinkMacSystemFont, sans-serif";
      context.textAlign = "center";
      context.textBaseline = "middle";
      context.fillText(String(shape.number), shape.at.x, shape.at.y + 0.5);
      break;
    }
  }

  context.restore();
}

function strokePath(
  context: CanvasRenderingContext2D,
  points: readonly Point[],
  straight: boolean,
): void {
  context.beginPath();
  context.moveTo(points[0].x, points[0].y);
  if (straight) {
    const last = points[points.length - 1];
    context.lineTo(last.x, last.y);
  } else {
    for (let index = 1; index < points.length; index += 1) {
      context.lineTo(points[index].x, points[index].y);
    }
  }
  context.stroke();
}

type ArrowHead = {
  tip: Point;
  base: Point;
  /** Half the width of the back edge, in points. */
  half: number;
  /** Where the shaft must stop so its round cap stays inside the head. */
  shaftEnd: Point;
  angle: number;
};

function arrowHead(points: readonly Point[], width: number): ArrowHead {
  const tip = points[points.length - 1];
  const previous = points[points.length - 2] ?? points[0];
  const angle = Math.atan2(tip.y - previous.y, tip.x - previous.x);
  const length = Math.max(12, width * 4.5);
  const along = (distance: number) => ({
    x: tip.x - Math.cos(angle) * distance,
    y: tip.y - Math.sin(angle) * distance,
  });
  return {
    tip,
    base: along(length),
    half: length * 0.42,
    shaftEnd: along(length * 0.72),
    angle,
  };
}

function fillArrowHead(context: CanvasRenderingContext2D, head: ArrowHead): void {
  const spreadX = Math.cos(head.angle + Math.PI / 2) * head.half;
  const spreadY = Math.sin(head.angle + Math.PI / 2) * head.half;
  context.beginPath();
  context.moveTo(head.tip.x, head.tip.y);
  context.lineTo(head.base.x + spreadX, head.base.y + spreadY);
  context.lineTo(head.base.x - spreadX, head.base.y - spreadY);
  context.closePath();
  context.fill();
}

/** Block average of what is already on the canvas: the original pixels are gone. */
export function pixelate(
  context: CanvasRenderingContext2D,
  rect: Rect,
  scale: number,
  block: number,
): void {
  const width = Math.max(1, Math.round((rect.w * scale) / block));
  const height = Math.max(1, Math.round((rect.h * scale) / block));
  const source = sourcePixels(context, rect, scale);
  if (!source) return;

  const small = document.createElement("canvas");
  small.width = width;
  small.height = height;
  const smallContext = small.getContext("2d");
  if (!smallContext) return;
  smallContext.drawImage(
    source.canvas,
    source.x,
    source.y,
    source.w,
    source.h,
    0,
    0,
    width,
    height,
  );

  context.save();
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.imageSmoothingEnabled = false;
  context.drawImage(
    small,
    0,
    0,
    width,
    height,
    Math.round(rect.x * scale),
    Math.round(rect.y * scale),
    Math.round(rect.w * scale),
    Math.round(rect.h * scale),
  );
  context.restore();
}

type Source = { canvas: HTMLCanvasElement; x: number; y: number; w: number; h: number };

function sourcePixels(
  context: CanvasRenderingContext2D,
  rect: Rect,
  scale: number,
): Source | null {
  const canvas = context.canvas;
  const x = Math.max(0, Math.round(rect.x * scale));
  const y = Math.max(0, Math.round(rect.y * scale));
  const w = Math.min(canvas.width - x, Math.round(rect.w * scale));
  const h = Math.min(canvas.height - y, Math.round(rect.h * scale));
  if (w <= 0 || h <= 0) return null;
  return { canvas, x, y, w, h };
}
