/**
 * Annotation model: an ordered list of shapes plus an undo stack.
 *
 * Pure data — no canvas, no DOM — so it can be unit tested in Node and reused by the
 * overlay. Coordinates are in **points** (the same space as mouse events); the
 * renderer scales them by the display factor when exporting, which is what keeps 1x
 * and 2x exports visually identical (spec/05-annotation.md §8).
 */

export type Point = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };

export type ToolId =
  | "select"
  | "rect"
  | "ellipse"
  | "arrow"
  | "pen"
  | "marker"
  | "mosaic"
  | "text"
  | "counter";

export type Style = {
  color: string;
  width: number;
  /** 0–1, used by the marker. */
  alpha?: number;
};

export type Shape =
  | { id: number; kind: "rect"; rect: Rect; style: Style }
  | { id: number; kind: "ellipse"; rect: Rect; style: Style }
  | { id: number; kind: "line"; points: Point[]; style: Style; arrow: boolean }
  | { id: number; kind: "pen"; points: Point[]; style: Style }
  | { id: number; kind: "marker"; points: Point[]; style: Style }
  | { id: number; kind: "mosaic"; rect: Rect; block: number }
  | {
      id: number;
      kind: "text";
      at: Point;
      text: string;
      size: number;
      style: Style;
    }
  | { id: number; kind: "counter"; at: Point; number: number; style: Style };

type Command =
  | { type: "add"; shape: Shape }
  | { type: "remove"; shape: Shape; index: number }
  | { type: "modify"; before: Shape; after: Shape };

const UNDO_LIMIT = 200;

/** Normalised rectangle from two dragged corners (any direction). */
export function rectFromCorners(a: Point, b: Point): Rect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.abs(a.x - b.x),
    h: Math.abs(a.y - b.y),
  };
}

export function rectIsEmpty(rect: Rect, minimum = 1): boolean {
  return rect.w < minimum || rect.h < minimum;
}

export class SceneModel {
  private shapes: Shape[] = [];
  private undoStack: Command[] = [];
  private redoStack: Command[] = [];
  private nextId = 1;
  private nextCounter = 1;

  get all(): readonly Shape[] {
    return this.shapes;
  }

  get size(): number {
    return this.shapes.length;
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  id(): number {
    return this.nextId++;
  }

  /** The most recently added shape of one of these kinds (used by the size controls). */
  lastWithKinds(kinds: readonly Shape["kind"][]): Shape | null {
    for (let index = this.shapes.length - 1; index >= 0; index -= 1) {
      if (kinds.includes(this.shapes[index].kind)) return this.shapes[index];
    }
    return null;
  }

  byId(id: number): Shape | null {
    return this.shapes.find((shape) => shape.id === id) ?? null;
  }

  /** Replaces a shape without recording undo (used while dragging, which records once). */
  replaceShape(shape: Shape): void {
    const index = this.shapes.findIndex((entry) => entry.id === shape.id);
    if (index >= 0) this.shapes[index] = shape;
  }

  /** Changes one shape's pen style by id (the shape the user has selected). */
  updateStyle(id: number, patch: Partial<Style>): boolean {
    const index = this.shapes.findIndex((shape) => shape.id === id);
    if (index < 0) return false;
    const before = this.shapes[index];
    if (!("style" in before)) return false;
    const style = { ...before.style, ...patch };
    if (style.color === before.style.color && style.width === before.style.width) {
      return false;
    }
    const after = { ...before, style } as Shape;
    this.shapes[index] = after;
    this.push({ type: "modify", before, after });
    return true;
  }

  /** Changes a label's font size (the size control means "字号" for text). */
  updateTextSize(id: number, size: number): boolean {
    const index = this.shapes.findIndex((shape) => shape.id === id);
    if (index < 0) return false;
    const before = this.shapes[index];
    if (before.kind !== "text" || before.size === size) return false;
    const after = { ...before, size } as Shape;
    this.shapes[index] = after;
    this.push({ type: "modify", before, after });
    return true;
  }

  /** Rewrites the text of an existing label (double-click to edit). */
  updateText(id: number, text: string): boolean {
    const index = this.shapes.findIndex((shape) => shape.id === id);
    if (index < 0) return false;
    const before = this.shapes[index];
    if (before.kind !== "text" || before.text === text) return false;
    const after = { ...before, text } as Shape;
    this.shapes[index] = after;
    this.push({ type: "modify", before, after });
    return true;
  }

  /** Records an already-applied change so a drag can be undone in one step. */
  recordModify(before: Shape, after: Shape): void {
    this.push({ type: "modify", before, after });
  }

  /** Sequence number for the ①②③ tool; explicit so undo does not rewind it. */
  counter(): number {
    return this.nextCounter++;
  }

  resetCounter(): void {
    this.nextCounter = 1;
  }

  add(shape: Shape): Shape {
    this.shapes.push(shape);
    this.push({ type: "add", shape });
    return shape;
  }

  remove(id: number): boolean {
    const index = this.shapes.findIndex((shape) => shape.id === id);
    if (index < 0) return false;
    const [shape] = this.shapes.splice(index, 1);
    this.push({ type: "remove", shape, index });
    return true;
  }

  replace(next: Shape): boolean {
    const index = this.shapes.findIndex((shape) => shape.id === next.id);
    if (index < 0) return false;
    const before = this.shapes[index];
    this.shapes[index] = next;
    this.push({ type: "modify", before, after: next });
    return true;
  }

  clear(): void {
    if (this.shapes.length === 0) return;
    const removed = [...this.shapes];
    this.shapes = [];
    this.undoStack = [];
    this.redoStack = [];
    // Clearing is not undoable on purpose: Esc means "start over", and a stale
    // history after that is more confusing than helpful.
    void removed;
  }

  undo(): boolean {
    const command = this.undoStack.pop();
    if (!command) return false;
    switch (command.type) {
      case "add": {
        const index = this.shapes.findIndex((shape) => shape.id === command.shape.id);
        if (index >= 0) this.shapes.splice(index, 1);
        break;
      }
      case "remove":
        this.shapes.splice(
          Math.min(command.index, this.shapes.length),
          0,
          command.shape,
        );
        break;
      case "modify": {
        const index = this.shapes.findIndex((shape) => shape.id === command.before.id);
        if (index >= 0) this.shapes[index] = command.before;
        break;
      }
    }
    this.redoStack.push(command);
    return true;
  }

  redo(): boolean {
    const command = this.redoStack.pop();
    if (!command) return false;
    switch (command.type) {
      case "add":
        this.shapes.push(command.shape);
        break;
      case "remove": {
        const index = this.shapes.findIndex((shape) => shape.id === command.shape.id);
        if (index >= 0) this.shapes.splice(index, 1);
        break;
      }
      case "modify": {
        const index = this.shapes.findIndex((shape) => shape.id === command.after.id);
        if (index >= 0) this.shapes[index] = command.after;
        break;
      }
    }
    this.undoStack.push(command);
    return true;
  }

  /** Shapes under the point, topmost first. */
  hitTest(point: Point, tolerance = 4): Shape | null {
    for (let index = this.shapes.length - 1; index >= 0; index -= 1) {
      if (hitsShape(this.shapes[index], point, tolerance)) return this.shapes[index];
    }
    return null;
  }

  private push(command: Command): void {
    this.undoStack.push(command);
    if (this.undoStack.length > UNDO_LIMIT) this.undoStack.shift();
    this.redoStack = [];
  }
}

/** The same shape moved by (dx, dy). */
export function shiftShape(shape: Shape, dx: number, dy: number): Shape {
  const movePoint = (point: Point) => ({ x: point.x + dx, y: point.y + dy });
  switch (shape.kind) {
    case "rect":
    case "ellipse":
    case "mosaic":
      return {
        ...shape,
        rect: { ...shape.rect, x: shape.rect.x + dx, y: shape.rect.y + dy },
      };
    case "line":
    case "pen":
    case "marker":
      return { ...shape, points: shape.points.map(movePoint) };
    case "text":
    case "counter":
      return { ...shape, at: movePoint(shape.at) };
  }
}

export function boundsOf(shape: Shape, tolerance = 0): Rect[] {
  switch (shape.kind) {
    case "rect":
    case "ellipse":
    case "mosaic":
      return [
        inflate(
          shape.rect,
          tolerance + (shape.kind === "rect" ? (shape.style.width ?? 0) / 2 : 0),
        ),
      ];
    case "line":
    case "pen":
    case "marker":
      return shape.points.map((point) => ({
        x: point.x - tolerance - shape.style.width / 2,
        y: point.y - tolerance - shape.style.width / 2,
        w: tolerance * 2 + shape.style.width,
        h: tolerance * 2 + shape.style.width,
      }));
    case "text": {
      // Estimate the drawn width per character class: a CJK glyph is about one em wide,
      // Latin about 0.58 em. The old flat 0.62 em made the box far too narrow for
      // Chinese text, so clicking the right half of a label missed it.
      let em = 0;
      for (const character of shape.text) {
        em += /[\u2e80-\u9fff\uf900-\ufaff\uff00-\uffef\u3000-\u303f]/.test(character)
          ? 1
          : 0.58;
      }
      return [
        {
          x: shape.at.x - tolerance,
          y: shape.at.y - shape.size - tolerance,
          w: shape.size * em + tolerance * 2,
          h: shape.size * 1.35 + tolerance * 2,
        },
      ];
    }
    case "counter":
      return [
        {
          x: shape.at.x - 14 - tolerance,
          y: shape.at.y - 14 - tolerance,
          w: 28 + tolerance * 2,
          h: 28 + tolerance * 2,
        },
      ];
  }
}

/**
 * Whether the point is on a shape. Brush strokes are tested by distance to the line
 * they draw, not by their bounding box: an arrow across the screen has a huge box, and
 * selecting it from an empty corner felt like a bug.
 */
export function hitsShape(shape: Shape, point: Point, tolerance = 0): boolean {
  switch (shape.kind) {
    case "line":
    case "pen":
    case "marker": {
      const reach = tolerance + (shape.style.width * (shape.kind === "marker" ? 3 : 1)) / 2;
      const points = shape.points;
      if (points.length === 0) return false;
      if (points.length === 1) return distance(point, points[0]) <= reach;
      for (let index = 1; index < points.length; index += 1) {
        if (distanceToSegment(point, points[index - 1], points[index]) <= reach) {
          return true;
        }
      }
      return false;
    }
    default:
      return boundsOf(shape, tolerance).some((rect) => contains(rect, point));
  }
}

function distance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function distanceToSegment(point: Point, start: Point, end: Point): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return distance(point, start);
  // Project onto the segment, clamped to its ends.
  const t = Math.max(
    0,
    Math.min(1, ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared),
  );
  return distance(point, { x: start.x + t * dx, y: start.y + t * dy });
}

function inflate(rect: Rect, by: number): Rect {
  return { x: rect.x - by, y: rect.y - by, w: rect.w + by * 2, h: rect.h + by * 2 };
}

function contains(rect: Rect, point: Point): boolean {
  return (
    point.x >= rect.x &&
    point.y >= rect.y &&
    point.x <= rect.x + rect.w &&
    point.y <= rect.y + rect.h
  );
}
