import type { ObjectUpdate, SessionObject } from "./types";

export function cloneSessionObjects(objects: SessionObject[]): SessionObject[] {
  return objects.map((object) => ({ ...object }));
}

export function applyObjectUpdates(prev: SessionObject[], updates: ObjectUpdate[]): SessionObject[] {
  if (updates.length === 0) return prev;

  let next = cloneSessionObjects(prev);
  for (const update of updates) {
    if (update.action === "upsert") {
      next = [update.object, ...next.filter((object) => object.handle !== update.object.handle)];
    } else if (update.action === "remove") {
      next = next.filter((object) => object.handle !== update.handle);
    } else if (update.action === "remove_last") {
      next = next.slice(1);
    }
  }
  return next;
}

export function mergeSessionObjects(prev: SessionObject[], incoming: SessionObject[]): SessionObject[] {
  if (incoming.length === 0) return prev;
  const incomingHandles = new Set(incoming.map((object) => object.handle));
  return [...incoming, ...prev.filter((object) => !incomingHandles.has(object.handle))];
}

function kindReferenceName(kind: string): string {
  switch (kind) {
    case "LINE":
      return "条直线";
    case "CIRCLE":
      return "个圆";
    case "ARC":
      return "段圆弧";
    case "LWPOLYLINE":
      return "条多段线";
    default:
      return `个${kind}对象`;
  }
}

export function sourceDisplayLabel(source?: string): string {
  switch (source) {
    case "generated":
      return "创建";
    case "selection":
      return "导入";
    default:
      return "纳入";
  }
}

export function getObjectReferenceHints(objects: SessionObject[]): Map<string, string[]> {
  const hints = new Map<string, string[]>();
  const kindCounts = new Map<string, number>();
  const sourceCounts = new Map<string, number>();

  objects.forEach((object, idx) => {
    const nextKindCount = (kindCounts.get(object.kind) ?? 0) + 1;
    kindCounts.set(object.kind, nextKindCount);

    const sourceKey = object.source ?? "session";
    const nextSourceCount = (sourceCounts.get(sourceKey) ?? 0) + 1;
    sourceCounts.set(sourceKey, nextSourceCount);

    const nextHints = [`第${idx + 1}个对象`, `第${nextKindCount}${kindReferenceName(object.kind)}`];
    if (idx === 0) nextHints.push("最新对象");
    if (nextKindCount === 1) nextHints.push(`最新${kindReferenceName(object.kind)}`);

    const sourceLabel = sourceDisplayLabel(object.source);
    nextHints.push(`第${nextSourceCount}个${sourceLabel}对象`);
    if (nextSourceCount === 1) nextHints.push(`最新${sourceLabel}的对象`);

    hints.set(object.handle, nextHints);
  });

  return hints;
}
