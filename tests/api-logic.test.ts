// 前端纯逻辑测试：待处理计数口径与落盘配对
//
// 项目没有前端测试框架，也不想为一个文件引入新依赖。
// Node 22 自带 TypeScript 类型剥离，可以直接 import .ts 并断言。
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import {
  countPending,
  placements,
  type MySkillItem,
  type UpdateCheckItem,
  type UpdateStatus,
} from "../src/api.ts";

function item(
  name: string,
  tools: string[],
  identity: MySkillItem["identity"],
  targetPaths?: string[],
): MySkillItem {
  return {
    name,
    tools,
    targetPaths: targetPaths ?? tools.map((t) => `/home/u/.${t}/skills/${name}`),
    description: "",
    version: null,
    identity,
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
  };
}

test("placements 按索引配对，不依赖 indexOf 反查", () => {
  const it = item("x", ["workbuddy", "qwenwork"], "managed", ["/p/a", "/p/b"]);
  assert.deepEqual(placements(it), [
    { tool: "workbuddy", targetPath: "/p/a" },
    { tool: "qwenwork", targetPath: "/p/b" },
  ]);
});

test("placements 在数组短于 tools 时不给错路径", () => {
  // 后端保证同长，但前端不该在错位时静默取到别人的路径
  const it = item("x", ["a", "b"], "managed", ["/only-one"]);
  const got = placements(it);
  assert.equal(got[0].targetPath, "/only-one");
  assert.equal(got[1].targetPath, "", "缺失的路径应为空，而不是回退到第一个");
});

test("local 技能计入待处理，external 不计", () => {
  const items = [
    item("local-skill", ["claude"], "local"),
    item("ext-skill", ["codex"], "external"),
  ];
  assert.equal(countPending(items, []), 1);
});

test("managed 按需处理状态计数，且看每个工具", () => {
  // grill-me 装在两个工具下，只有其中一个需要更新
  const items = [item("grill-me", ["workbuddy", "qwenwork"], "managed")];
  const updates = [
    upd("grill-me", "workbuddy", "uptodate"),
    upd("grill-me", "qwenwork", "updateavailable"),
  ];
  assert.equal(countPending(items, updates), 1, "一个技能只算一条");
});

test("同名技能不同工具的状态不会互相串", () => {
  const items = [
    item("x", ["claude"], "managed"),
    item("y", ["claude"], "managed"),
  ];
  // 名字不同但状态都给到 x 的工具上，y 应为未检查
  const updates = [upd("x", "claude", "conflict")];
  assert.equal(countPending(items, updates), 1);
});

test("四种需处理状态都计入", () => {
  const statuses: UpdateStatus[] = [
    "updateavailable",
    "conflict",
    "locallymodified",
    "localmissing",
  ];
  statuses.forEach((s) => {
    const items = [item("s", ["claude"], "managed")];
    assert.equal(
      countPending(items, [upd("s", "claude", s)]),
      1,
      `${s} 应计入待处理`,
    );
  });
});

test("uptodate 与未检查都不计入", () => {
  const items = [item("a", ["claude"], "managed"), item("b", ["claude"], "managed")];
  assert.equal(countPending(items, [upd("a", "claude", "uptodate")]), 0);
});

test("同一份输入在任何调用下结果稳定", () => {
  // 侧边栏与页内都调这一个函数，只要输入相同数字必然相同
  const items = [
    item("a", ["claude"], "local"),
    item("b", ["codex", "agents"], "managed"),
  ];
  const updates = [upd("b", "agents", "conflict")];
  assert.equal(countPending(items, updates), countPending(items, updates));
  assert.equal(countPending(items, updates), 2);
});
