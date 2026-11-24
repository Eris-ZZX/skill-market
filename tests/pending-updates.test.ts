// 「检查更新」弹窗的列表筛选
//
// pendingUpdates 决定弹窗里列什么。矩阵行内的「更新」按钮与这个弹窗
// 走同一个 update()，这里只管「哪些记录该出现在列表里」。
//
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import {
  pendingUpdates,
  type MySkillItem,
  type UpdateCheckItem,
  type UpdateStatus,
} from "../src/api.ts";

function item(
  name: string,
  tools: string[],
  opts: { external?: boolean; remote?: boolean } = {},
): MySkillItem {
  return {
    name,
    tools,
    targetPaths: tools.map((t) => `/home/u/.${t}/skills/${name}`),
    description: "",
    version: null,
    identity: opts.external ? "external" : "installed",
    externallyManaged: opts.external ?? false,
    hasRemoteSource: opts.remote ?? true,
    status: null,
    installedVersion: null,
  };
}

function upd(
  name: string,
  tool: string,
  status: UpdateStatus,
  ignored = false,
): UpdateCheckItem {
  return {
    name,
    tool,
    status,
    installedVersion: "1.0.0",
    remoteVersion: "1.1.0",
    ignored,
  };
}

test("只返回 updateavailable / conflict / locallymodified", () => {
  const items = [item("x", ["claude", "cursor", "codex", "agents", "qwenwork"])];
  const updates = [
    upd("x", "claude", "updateavailable"),
    upd("x", "cursor", "conflict"),
    upd("x", "codex", "locallymodified"),
    upd("x", "agents", "uptodate"),
    upd("x", "qwenwork", "remotemissing"),
  ];
  const rows = pendingUpdates(items, updates);
  assert.deepEqual(
    rows.map((r) => [r.tool, r.status]),
    [
      ["claude", "updateavailable"],
      ["cursor", "conflict"],
      ["codex", "locallymodified"],
    ],
    "只看三种状态，uptodate / remotemissing 不进列表",
  );
});

test("无来源的技能不计入", () => {
  const items = [item("no-src", ["claude"], { remote: false })];
  const rows = pendingUpdates(items, [upd("no-src", "claude", "updateavailable")]);
  assert.deepEqual(rows, [], "没有远端可比就没有「更新」这回事");
});

test("外部管理的技能不计入", () => {
  const items = [item("ext", ["claude"], { external: true })];
  const rows = pendingUpdates(items, [upd("ext", "claude", "updateavailable")]);
  assert.deepEqual(rows, [], "替外部工具升级会动到别人的东西");
});

test("空结果返回空数组（空技能或空更新）", () => {
  assert.deepEqual(pendingUpdates([], []), []);
  // 有技能但检查结果为空：什么都没装/没检查过
  assert.deepEqual(pendingUpdates([item("x", ["claude"])], []), []);
});

test("ignored 的记录排除，不受用户关注的干扰", () => {
  const items = [item("x", ["claude"])];
  const rows = pendingUpdates(items, [upd("x", "claude", "updateavailable", true)]);
  assert.deepEqual(rows, []);
});

test("带出版本号，供弹窗显示 当前 → 远端", () => {
  const rows = pendingUpdates(
    [item("x", ["claude"])],
    [upd("x", "claude", "updateavailable")],
  );
  assert.equal(rows[0].installedVersion, "1.0.0");
  assert.equal(rows[0].remoteVersion, "1.1.0");
});

test("按技能摊平到每个工具，同名不同工具各自一条", () => {
  const items = [item("x", ["claude", "cursor"])];
  const rows = pendingUpdates(items, [
    upd("x", "claude", "updateavailable"),
    upd("x", "cursor", "updateavailable"),
  ]);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows.map((r) => r.tool), ["claude", "cursor"]);
});
