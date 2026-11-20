// 状态/工具/身份筛选
//
// applyFilters 的返回值同时是 #56 批量操作的作用域，所以这份筛选逻辑
// 必须只有一处实现——页面里另写近似版本会让「批量改到看不见的技能」。
//
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import {
  applyFilters,
  EMPTY_FILTERS,
  type FilterState,
  type MySkillItem,
  type UpdateCheckItem,
  type UpdateStatus,
} from "../src/api.ts";

function item(
  name: string,
  tools: string[],
  identity: MySkillItem["identity"] = "managed",
  description = "",
): MySkillItem {
  return {
    name,
    tools,
    targetPaths: tools.map((t) => `/home/u/.${t}/skills/${name}`),
    description,
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
    ignored: false,
  };
}

const f = (patch: Partial<FilterState>): FilterState => ({
  ...EMPTY_FILTERS,
  ...patch,
});

// ========== 无筛选 ==========

test("无筛选时返回全部", () => {
  const items = [item("a", ["cursor"]), item("b", []), item("c", ["claude"])];
  assert.deepEqual(
    applyFilters(items, [], EMPTY_FILTERS).map((i) => i.name),
    ["a", "b", "c"],
  );
});

test("空列表不崩", () => {
  assert.deepEqual(applyFilters([], [], EMPTY_FILTERS), []);
  assert.deepEqual(applyFilters([], [], f({ status: "updates" })), []);
});

// ========== 状态 ==========

test("status=updates 只看有可更新的", () => {
  const items = [item("a", ["cursor"]), item("b", ["cursor"]), item("c", ["cursor"])];
  const updates = [
    upd("a", "cursor", "updateavailable"),
    upd("b", "cursor", "uptodate"),
    upd("c", "cursor", "locallymodified"),
  ];
  assert.deepEqual(
    applyFilters(items, updates, f({ status: "updates" })).map((i) => i.name),
    ["a"],
  );
});

test("status=local-modified 与 conflict 各自独立", () => {
  const items = [item("a", ["cursor"]), item("b", ["cursor"]), item("c", ["cursor"])];
  const updates = [
    upd("a", "cursor", "locallymodified"),
    upd("b", "cursor", "conflict"),
    upd("c", "cursor", "uptodate"),
  ];
  assert.deepEqual(
    applyFilters(items, updates, f({ status: "local-modified" })).map((i) => i.name),
    ["a"],
  );
  assert.deepEqual(
    applyFilters(items, updates, f({ status: "conflict" })).map((i) => i.name),
    ["b"],
  );
});

test("status=none 取所有状态都最新的，无记录也算", () => {
  const items = [
    item("a", ["cursor"]),
    item("b", ["cursor"]),
    item("c", []), // 一个工具都没装 = 没有状态记录
  ];
  const updates = [
    upd("a", "cursor", "uptodate"),
    upd("b", "cursor", "updateavailable"),
  ];
  assert.deepEqual(
    applyFilters(items, updates, f({ status: "none" })).map((i) => i.name),
    ["a", "c"],
  );
});

test("状态看任一工具：装多处只要一处可更新就该出现", () => {
  const items = [item("a", ["cursor", "claude"])];
  const updates = [
    upd("a", "cursor", "uptodate"),
    upd("a", "claude", "updateavailable"),
  ];
  assert.deepEqual(
    applyFilters(items, updates, f({ status: "updates" })).map((i) => i.name),
    ["a"],
    "claude 侧有更新就该命中",
  );
});

// ========== 工具 ==========

test("tool 筛选只看装在该工具下的", () => {
  const items = [
    item("a", ["cursor"]),
    item("b", ["claude"]),
    item("c", ["cursor", "claude"]),
  ];
  assert.deepEqual(
    applyFilters(items, [], f({ tool: "cursor" })).map((i) => i.name),
    ["a", "c"],
  );
});

// ========== 身份 ==========

test("identity 筛选", () => {
  const items = [
    item("a", ["cursor"], "managed"),
    item("b", ["cursor"], "external"),
    item("c", [], "local"),
  ];
  assert.deepEqual(
    applyFilters(items, [], f({ identity: "external" })).map((i) => i.name),
    ["b"],
  );
  assert.deepEqual(
    applyFilters(items, [], f({ identity: "local" })).map((i) => i.name),
    ["c"],
  );
});

// ========== 叠加 ==========

test("多条件是 AND", () => {
  const items = [
    item("a", ["cursor"], "managed"),
    item("b", ["cursor"], "local"),
    item("c", ["claude"], "managed"),
  ];
  const updates = [
    upd("a", "cursor", "updateavailable"),
    upd("b", "cursor", "updateavailable"),
  ];
  // 同时要求：有更新 + 装在 cursor + 本工具管理
  assert.deepEqual(
    applyFilters(items, updates, f({ status: "updates", tool: "cursor", identity: "managed" }))
      .map((i) => i.name),
    ["a"],
  );
});

test("query 与其它条件叠加", () => {
  const items = [
    item("grill-me", ["cursor"], "managed", "烧烤"),
    item("grill-me-x", ["cursor"], "external", "别的"),
    item("dws", ["cursor"], "managed", "报表"),
  ];
  const r = applyFilters(items, [], f({ query: "grill", identity: "managed" }));
  assert.deepEqual(r.map((i) => i.name), ["grill-me"], "query 与 identity 要同时满足");
});

test("query 匹配描述也能命中", () => {
  const items = [item("dws", ["cursor"], "managed", "生成销售报表")];
  assert.deepEqual(
    applyFilters(items, [], f({ query: "报表" })).map((i) => i.name),
    ["dws"],
  );
});

test("条件无交集时返回空数组而不是报错", () => {
  const items = [item("a", ["cursor"], "managed")];
  assert.deepEqual(applyFilters(items, [], f({ tool: "claude" })), []);
});
