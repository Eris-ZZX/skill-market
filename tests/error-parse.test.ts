// 前端错误解析测试
//
// 后端四个高频命令现在返回结构化 JSON，其它命令仍是纯文本。
// parseError 必须两种都能处理——否则一个格式变化就会让错误显示成空白。
//
// 运行：node --experimental-strip-types --test tests/*.test.ts

import test from "node:test";
import assert from "node:assert/strict";
import { parseError, ACTION_LABEL } from "../src/api.ts";

test("合法结构化 JSON 被完整解析", () => {
  const raw = JSON.stringify({
    code: "AUTH_INVALID",
    message: "访问令牌无效或已过期",
    detail: "HTTP 401 Unauthorized",
    action: "open_settings",
  });
  const e = parseError(raw);
  assert.equal(e.code, "AUTH_INVALID");
  assert.equal(e.message, "访问令牌无效或已过期");
  assert.equal(e.detail, "HTTP 401 Unauthorized");
  assert.equal(e.action, "open_settings");
});

test("action 为 null 时保持 null", () => {
  const raw = JSON.stringify({
    code: "TOOL_NOT_INSTALLED",
    message: "目标工具未安装",
    detail: "千问办公 未安装",
    action: null,
  });
  assert.equal(parseError(raw).action, null);
});

test("纯文本退回 UNKNOWN 且原文进 detail", () => {
  const raw = "状态锁已损坏";
  const e = parseError(raw);
  assert.equal(e.code, "UNKNOWN");
  assert.equal(e.message, raw, "纯文本直接当标题，不要显示成「操作失败」");
  assert.equal(e.detail, raw, "原始文本必须保留");
  assert.equal(e.action, null);
});

test("空字符串不崩且给出兜底文案", () => {
  const e = parseError("");
  assert.equal(e.code, "UNKNOWN");
  assert.ok(e.message.length > 0, "空输入也要有可读文案");
});

test("null 与 undefined 不崩", () => {
  for (const v of [null, undefined]) {
    const e = parseError(v);
    assert.equal(e.code, "UNKNOWN");
    assert.ok(typeof e.message === "string");
  }
});

test("垃圾 JSON 不崩且退回纯文本", () => {
  // 语法坏掉
  assert.equal(parseError("{not json").code, "UNKNOWN");
  // 是合法 JSON 但不是对象
  assert.equal(parseError("123").code, "UNKNOWN");
  assert.equal(parseError("\"just a string\"").code, "UNKNOWN");
  // 是对象但缺必需字段
  const missing = JSON.stringify({ foo: "bar" });
  const e = parseError(missing);
  assert.equal(e.code, "UNKNOWN");
  assert.equal(e.detail, missing, "无法识别时应保留原文");
});

test("Error 实例（非字符串）也能解析", () => {
  // Tauri 的 invoke 在某些情况下抛 Error 对象而非字符串
  const e = parseError(new Error("连接超时"));
  assert.equal(e.code, "UNKNOWN");
  assert.ok(e.message.includes("连接超时"), "应取到 Error 的文本: " + e.message);
});

test("每个 action 都有中文按钮文案", () => {
  // 后端会出现这四种；漏一个界面就会显示 undefined
  for (const a of ["retry", "open_settings", "reinstall", "contact_it"]) {
    assert.ok(ACTION_LABEL[a], `缺少 ${a} 的按钮文案`);
  }
});
