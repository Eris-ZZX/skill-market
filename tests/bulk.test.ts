// 批量操作的作用域计算
//
// 关键在于「作用域跟随当前筛选」：用户筛了「只看本地已有」再做
// 「装上其余」，不应该连筛掉的技能一起动。
//
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import { resolveBulkScope, type MySkillItem } from "../src/api.ts";

function item(name: string, tools: string[]): MySkillItem {
  return {
    name,
    tools,
    targetPaths: tools.map((t) => `/home/u/.${t}/skills/${name}`),
    description: "",
    version: null,
    identity: "managed",
    status: null,
    installedVersion: null,
  };
}

test("无筛选时 targets 是全部未装的", () => {
  const items = [
    item("a", ["cursor"]),
    item("b", []),
    item("c", []),
  ];
  const scope = resolveBulkScope(items, "cursor", true);
  assert.deepEqual(scope.targets, ["b", "c"], "只有未装的才要装");
  assert.deepEqual(scope.alreadyDone, ["a"]);
});

test("有筛选时只作用于筛选结果", () => {
  // 调用方传进来的是筛选后的列表 —— 筛掉的技能根本不在这个数组里
  const filtered = [item("b", []), item("c", [])];
  const scope = resolveBulkScope(filtered, "cursor", true);
  assert.deepEqual(scope.targets, ["b", "c"]);
  assert.equal(scope.alreadyDone.length, 0);
});

test("已全装时 targets 为空、alreadyDone 有值", () => {
  const items = [item("a", ["cursor"]), item("b", ["cursor"])];
  const scope = resolveBulkScope(items, "cursor", true);
  assert.deepEqual(scope.targets, []);
  assert.deepEqual(scope.alreadyDone, ["a", "b"]);
});

test("全部已装时卸载作用域正确", () => {
  const items = [item("a", ["cursor"]), item("b", ["cursor"])];
  const scope = resolveBulkScope(items, "cursor", false);
  assert.deepEqual(scope.targets, ["a", "b"], "全装了就全都要卸");
  assert.deepEqual(scope.alreadyDone, []);
});

test("部分装时卸载只针对已装的", () => {
  const items = [
    item("a", ["cursor"]),
    item("b", []),
    item("c", ["cursor", "claude"]),
  ];
  const scope = resolveBulkScope(items, "cursor", false);
  assert.deepEqual(scope.targets, ["a", "c"], "未装 b 不该被卸");
  assert.deepEqual(scope.alreadyDone, ["b"]);
});

test("空列表不崩且两边都为空", () => {
  const scope = resolveBulkScope([], "cursor", true);
  assert.deepEqual(scope.targets, []);
  assert.deepEqual(scope.alreadyDone, []);
});

test("别的工具装了不算这个工具装了", () => {
  // 工具维度必须严格区分：claude 装了不代表 cursor 装了
  const items = [item("a", ["claude"])];
  const scope = resolveBulkScope(items, "cursor", true);
  assert.deepEqual(scope.targets, ["a"], "claude 装着不代表 cursor 装着");
});

test("同一技能装多个工具时按目标工具判断", () => {
  const items = [
    item("a", ["cursor", "claude"]),
    item("b", ["claude"]),
  ];
  const forCursor = resolveBulkScope(items, "cursor", false);
  assert.deepEqual(forCursor.targets, ["a"], "只有 a 装了 cursor");
  const forClaude = resolveBulkScope(items, "claude", false);
  assert.deepEqual(forClaude.targets, ["a", "b"], "两个都装了 claude");
});
