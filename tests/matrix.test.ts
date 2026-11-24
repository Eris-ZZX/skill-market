// 矩阵视图的格子状态映射
//
// matrixCells 把一个技能摊成「每工具一格」。列由调用方给定（只传本机已装的
// 工具），没在该技能的 tools 里的工具就是未装。
//
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import {
  matrixCells,
  type MySkillItem,
  type UpdateCheckItem,
  type UpdateStatus,
} from "../src/api.ts";

function item(
  name: string,
  tools: string[],
  targetPaths?: string[],
): MySkillItem {
  return {
    name,
    tools,
    targetPaths: targetPaths ?? tools.map((t) => `/home/u/.${t}/skills/${name}`),
    description: "",
    version: null,
    identity: "installed",
    externallyManaged: false,
    hasRemoteSource: true,
    status: null,
    installedVersion: null,
  };
}

function upd(name: string, tool: string, status: UpdateStatus): UpdateCheckItem {
  return {
    name,
    tool,
    status,
    installedVersion: null,
    remoteVersion: null,
    ignored: false,
  };
}

test("工具不在该技能下 → installed 为 false 且无 targetPath", () => {
  const cells = matrixCells(item("x", ["claude"]), ["claude", "cursor"], []);
  const cursor = cells.find((c) => c.tool === "cursor")!;
  assert.equal(cursor.installed, false);
  assert.equal(cursor.targetPath, null, "未装时不该给路径");
  assert.equal(cursor.updateStatus, null);
});

test("工具在该技能下 → installed 为 true 且有 targetPath", () => {
  const cells = matrixCells(
    item("x", ["claude"], ["/p/claude/x"]),
    ["claude"],
    [],
  );
  assert.equal(cells[0].installed, true);
  assert.equal(cells[0].targetPath, "/p/claude/x");
});

test("多工具混合：装 2 个、未装 1 个", () => {
  const it = item("grill-me", ["claude", "agents"], ["/a", "/b"]);
  const cells = matrixCells(it, ["claude", "cursor", "agents"], []);
  assert.equal(cells.length, 3, "每个给定工具都有一格");
  assert.deepEqual(
    cells.map((c) => [c.tool, c.installed]),
    [
      ["claude", true],
      ["cursor", false],
      ["agents", true],
    ],
  );
  // 路径按索引对应各自的工具，不是回退到第一个
  assert.equal(cells[0].targetPath, "/a");
  assert.equal(cells[2].targetPath, "/b");
});

test("空 tools 数组不崩且全为未装", () => {
  const cells = matrixCells(item("x", []), ["claude", "cursor"], []);
  assert.equal(cells.length, 2);
  assert.ok(cells.every((c) => !c.installed));
});

test("空工具列返回空数组", () => {
  assert.deepEqual(matrixCells(item("x", ["claude"]), [], []), []);
});

test("updates 里没有对应记录时 updateStatus 为 null", () => {
  // 有安装记录但没有检查过更新
  const cells = matrixCells(item("x", ["claude"]), ["claude"], []);
  assert.equal(cells[0].installed, true);
  assert.equal(cells[0].updateStatus, null);
});

test("updates 有匹配记录时带出状态，且按 (name, tool) 匹配不串", () => {
  const it = item("x", ["claude", "cursor"], ["/a", "/b"]);
  const updates = [
    upd("x", "claude", "updateavailable"),
    // 同名但别的工具的状态不该被用到 claude 上
    upd("x", "cursor", "conflict"),
    // 别的技能的同名状态也不该被用到
    upd("y", "claude", "locallymodified"),
  ];
  const cells = matrixCells(it, ["claude", "cursor"], updates);
  assert.equal(cells[0].updateStatus, "updateavailable");
  assert.equal(cells[1].updateStatus, "conflict");
});

test("未装的格子即使 updates 里有记录也不带状态", () => {
  // 防御：残留的 update 记录不该让「未装」格看起来有状态
  const cells = matrixCells(
    item("x", []),
    ["claude"],
    [upd("x", "claude", "updateavailable")],
  );
  assert.equal(cells[0].installed, false);
  assert.equal(cells[0].updateStatus, null);
});

test("failedTools 标记对应格子", () => {
  const cells = matrixCells(
    item("x", ["claude"]),
    ["claude", "cursor"],
    [],
    ["cursor"],
  );
  assert.equal(cells.find((c) => c.tool === "cursor")!.failed, true);
  assert.equal(cells.find((c) => c.tool === "claude")!.failed, false);
});
