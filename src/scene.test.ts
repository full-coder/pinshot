import { describe, expect, it } from "vitest";
import { SceneModel, rectFromCorners, shiftShape, type Shape } from "./scene";

function rectShape(scene: SceneModel, x = 0, y = 0, w = 10, h = 10): Shape {
  return {
    id: scene.id(),
    kind: "rect",
    rect: { x, y, w, h },
    style: { color: "#e8452c", width: 2 },
  };
}

describe("rectFromCorners", () => {
  it("normalises any drag direction", () => {
    expect(rectFromCorners({ x: 100, y: 120 }, { x: 40, y: 20 })).toEqual({
      x: 40,
      y: 20,
      w: 60,
      h: 100,
    });
    expect(rectFromCorners({ x: 40, y: 20 }, { x: 100, y: 120 })).toEqual({
      x: 40,
      y: 20,
      w: 60,
      h: 100,
    });
  });
});

describe("SceneModel", () => {
  it("adds shapes in order and undoes them newest first", () => {
    const scene = new SceneModel();
    const first = scene.add(rectShape(scene, 0, 0));
    const second = scene.add(rectShape(scene, 20, 20));

    expect(scene.all.map((shape) => shape.id)).toEqual([first.id, second.id]);
    expect(scene.undo()).toBe(true);
    expect(scene.all.map((shape) => shape.id)).toEqual([first.id]);
    expect(scene.undo()).toBe(true);
    expect(scene.size).toBe(0);
    expect(scene.undo()).toBe(false);
  });

  it("redo restores the exact order", () => {
    const scene = new SceneModel();
    scene.add(rectShape(scene));
    const middle = scene.add(rectShape(scene, 5, 5));
    scene.add(rectShape(scene, 9, 9));
    scene.undo();
    scene.undo();
    expect(scene.size).toBe(1);
    expect(scene.redo()).toBe(true);
    expect(scene.all[1].id).toBe(middle.id);
    expect(scene.redo()).toBe(true);
    expect(scene.size).toBe(3);
  });

  it("removing then undoing puts the shape back at its old index", () => {
    const scene = new SceneModel();
    scene.add(rectShape(scene, 0, 0));
    const keep = scene.add(rectShape(scene, 10, 10));
    const tail = scene.add(rectShape(scene, 20, 20));

    expect(scene.remove(keep.id)).toBe(true);
    expect(scene.all.map((shape) => shape.id)).toEqual([1, tail.id]);
    expect(scene.undo()).toBe(true);
    expect(scene.all.map((shape) => shape.id)).toEqual([1, keep.id, tail.id]);
  });

  it("a new action clears the redo stack", () => {
    const scene = new SceneModel();
    scene.add(rectShape(scene));
    scene.undo();
    expect(scene.canRedo).toBe(true);
    scene.add(rectShape(scene, 50, 50));
    expect(scene.canRedo).toBe(false);
  });

  it("caps the undo history at 200 steps", () => {
    const scene = new SceneModel();
    for (let index = 0; index < 260; index += 1) scene.add(rectShape(scene, index, 0));
    let undone = 0;
    while (scene.undo()) undone += 1;
    expect(undone).toBe(200);
    expect(scene.size).toBe(60);
  });

  it("replaces a shape and can undo the change", () => {
    const scene = new SceneModel();
    const shape = scene.add(rectShape(scene, 0, 0));
    const moved: Shape = { ...shape, rect: { x: 40, y: 40, w: 10, h: 10 } } as Shape;
    expect(scene.replace(moved)).toBe(true);
    expect((scene.all[0] as Extract<Shape, { kind: "rect" }>).rect.x).toBe(40);
    scene.undo();
    expect((scene.all[0] as Extract<Shape, { kind: "rect" }>).rect.x).toBe(0);
  });

  it("hit tests the topmost shape first", () => {
    const scene = new SceneModel();
    scene.add(rectShape(scene, 0, 0, 100, 100));
    const top = scene.add(rectShape(scene, 10, 10, 20, 20));
    expect(scene.hitTest({ x: 15, y: 15 })?.id).toBe(top.id);
    expect(scene.hitTest({ x: 90, y: 90 })?.id).toBe(1);
    expect(scene.hitTest({ x: 500, y: 500 })).toBeNull();
  });

  it("keeps counter numbering stable across undo", () => {
    const scene = new SceneModel();
    expect(scene.counter()).toBe(1);
    expect(scene.counter()).toBe(2);
    scene.undo();
    expect(scene.counter()).toBe(3);
    scene.resetCounter();
    expect(scene.counter()).toBe(1);
  });

  it("ids never repeat", () => {
    const scene = new SceneModel();
    const ids = new Set<number>();
    for (let index = 0; index < 50; index += 1) ids.add(scene.id());
    expect(ids.size).toBe(50);
  });

});

describe("selection and restyling", () => {
  it("hit-tests a shape under the pointer", () => {
    const model = new SceneModel();
    const shape = rectShape(model, 100, 100, 80, 40);
    model.add(shape);
    expect(model.hitTest({ x: 120, y: 120 }, 4)?.id).toBe(shape.id);
    expect(model.hitTest({ x: 400, y: 400 }, 4)).toBeNull();
  });

  it("hit-tests Chinese labels across their full width", () => {
    const model = new SceneModel();
    const size = 21;
    const label = model.add({
      id: model.id(),
      kind: "text",
      at: { x: 100, y: 200 },
      text: "测试文字",
      size,
      style: { color: "#f00", width: 3 },
    });
    // Four CJK glyphs are about four em wide; the right half used to be missed.
    expect(model.hitTest({ x: 100 + size * 3.5, y: 200 - size * 0.5 }, 6)?.id).toBe(label.id);
    expect(model.hitTest({ x: 100 + size * 6, y: 200 - size * 0.5 }, 6)).toBeNull();
  });

  it("hit-tests a long arrow along its shaft, not its bounding box", () => {
    const model = new SceneModel();
    const arrow = model.add({
      id: model.id(),
      kind: "line",
      points: [
        { x: 0, y: 0 },
        { x: 400, y: 400 },
      ],
      style: { color: "#f00", width: 3 },
      arrow: true,
    });
    // Middle of the shaft, far from either end.
    expect(model.hitTest({ x: 200, y: 202 }, 6)?.id).toBe(arrow.id);
    // Inside the bounding box but nowhere near the line: must not select it.
    expect(model.hitTest({ x: 40, y: 360 }, 6)).toBeNull();
    expect(model.hitTest({ x: 405, y: 405 }, 6)?.id).toBe(arrow.id);
  });

  it("finds the most recent shape of a kind", () => {
    const model = new SceneModel();
    model.add(rectShape(model, 0, 0, 10, 10));
    const label = model.add({
      id: model.id(),
      kind: "text",
      at: { x: 10, y: 20 },
      text: "hi",
      size: 21,
      style: { color: "#f00", width: 3 },
    });
    const lastRect = model.add(rectShape(model, 40, 40, 10, 10));
    expect(model.lastWithKinds(["rect"])?.id).toBe(lastRect.id);
    expect(model.lastWithKinds(["text"])?.id).toBe(label.id);
    expect(model.lastWithKinds(["line"])).toBeNull();
  });

  it("changes only the shape that was asked for", () => {
    const model = new SceneModel();
    const first = model.add(rectShape(model, 10, 10, 40, 40));
    const second = model.add(rectShape(model, 80, 80, 40, 40));
    expect(model.updateStyle(first.id, { color: "#00ff00", width: 8 })).toBe(true);
    const a = model.byId(first.id);
    const b = model.byId(second.id);
    expect(a && "style" in a ? a.style.color : null).toBe("#00ff00");
    expect(b && "style" in b ? b.style.color : null).not.toBe("#00ff00");
    // One undo step restores it.
    model.undo();
    const restored = model.byId(first.id);
    expect(restored && "style" in restored ? restored.style.color : null).not.toBe("#00ff00");
  });

  it("moves a shape and undoes in one step", () => {
    const model = new SceneModel();
    const shape = model.add(rectShape(model, 10, 10, 40, 40));
    const before = model.byId(shape.id)!;
    const moved = shiftShape(before, 30, 20);
    model.replaceShape(moved);
    model.recordModify(before, moved);
    const after = model.byId(shape.id);
    expect(after && "rect" in after ? after.rect.x : 0).toBe(40);
    model.undo();
    const back = model.byId(shape.id);
    expect(back && "rect" in back ? back.rect.x : 0).toBe(10);
  });
});
