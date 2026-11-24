// 矩阵浮层的定位决策
//
// 两个浮层（列头弹层、行菜单）都改成绝对定位后，需要决定往哪边展开：
// 列头弹层按左右剩余空间挑一侧，行菜单按下边剩余空间上下翻。
// 这两处决策抽成纯函数，剩下的交给 CSS。
//
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import { menuSide, popoverSide } from "../src/api.ts";

// 列头浮层：320px 宽
const W = 320;

test("右边放得下就向右延展", () => {
  assert.equal(popoverSide(200, 400, W), "right");
});

test("右边放不下、左边放得下就向左延展（最后一列）", () => {
  assert.equal(popoverSide(500, 60, W), "left");
});

test("左边也放不下时选更宽敞的一侧，不让弹层完全出界", () => {
  // 两边都不够 320，右边更多 → 向右
  assert.equal(popoverSide(100, 260, W), "right");
  // 左边更多 → 向左
  assert.equal(popoverSide(260, 100, W), "left");
});

test("刚好等于宽度也算放得下", () => {
  assert.equal(popoverSide(0, W, W), "right");
  assert.equal(popoverSide(W, 0, W), "left");
});

test("第一列：左边没空间，必须向右", () => {
  assert.equal(popoverSide(36, 600, W), "right");
});

test("最后一列：右边没空间，必须向左", () => {
  assert.equal(popoverSide(600, 36, W), "left");
});

test("下方空间充足 → 向下弹", () => {
  assert.equal(menuSide(400, 84), "below");
});

test("下方空间不够 → 向上弹（最后一行）", () => {
  assert.equal(menuSide(40, 84), "above");
});

test("刚好等于菜单高度也要向上弹，留出余量不贴边", () => {
  assert.equal(menuSide(84, 84), "above");
  assert.equal(menuSide(91, 84), "above");
  assert.equal(menuSide(92, 84), "below");
});
