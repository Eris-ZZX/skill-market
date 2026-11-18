import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, AppSettings, PublishResult } from "../api";

interface Props {
  settings: AppSettings;
  onPublished: () => void;
}

const CATEGORIES = ["通用", "研发", "销售", "制造", "人事"];

export default function PublishPage({ settings, onPublished }: Props) {
  const [dir, setDir] = useState("");
  const [category, setCategory] = useState("通用");
  const [publisher, setPublisher] = useState(settings.publisherIdentity);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<PublishResult | null>(null);
  const [confirmed, setConfirmed] = useState(false);

  const pick = async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "选择技能目录（须包含 SKILL.md）",
      });
      if (typeof picked === "string") {
        setDir(picked);
        setResult(null);
        setError(null);
        setConfirmed(false);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await api.publishSkill({
        localDir: dir,
        category,
        publisher,
        message: message.trim() || null,
      });
      setResult(r);
      setConfirmed(false);
      onPublished();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = dir !== "" && publisher.trim() !== "" && confirmed;

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">发布技能</h1>
      </div>

      <div className="form">
        <div className="banner warn">
          <b>注意：发布直接写入 master</b>
          <br />
          本工具不设人工审核环节。发布后其他人立即能拉到新版本。
          发布前会自动保存当前版本的快照，万一有问题可在「我的技能」里回滚。
        </div>

        <div className="field">
          <label>技能目录</label>
          <div style={{ display: "flex", gap: 8 }}>
            <input
              value={dir}
              onChange={(e) => setDir(e.target.value)}
              placeholder="选择或粘贴包含 SKILL.md 的目录"
            />
            <button onClick={pick} style={{ flexShrink: 0 }}>
              浏览…
            </button>
          </div>
          <div className="hint">
            目录名建议与 SKILL.md 里的 name 一致。技能名要求小写字母、
            数字、连字符。
          </div>
        </div>

        <div className="field">
          <label>分类</label>
          <select value={category} onChange={(e) => setCategory(e.target.value)}>
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
          <div className="hint">
            决定技能落在 <code>skills/&lt;分类&gt;/&lt;技能名&gt;/</code>。
            已装用户不受分类变化影响（客户端按技能名识别）。
          </div>
        </div>

        <div className="field">
          <label>发布人</label>
          <input
            value={publisher}
            onChange={(e) => setPublisher(e.target.value)}
            placeholder="姓名/工号，如 张三/1234"
          />
          <div className="hint">
            会写进 commit message 和 SKILL.md 的 authors 字段。
            由于使用的是统一 Token，GitLab 只能看到机器人账号，
            所以这个署名是识别作者的唯一依据。
          </div>
        </div>

        <div className="field">
          <label>更新说明（可选）</label>
          <input
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="如：补充异常波动判断规则"
          />
        </div>

        <label className="check" style={{ marginTop: 4 }}>
          <input
            type="checkbox"
            checked={confirmed}
            onChange={(e) => setConfirmed(e.target.checked)}
          />
          我确认这个版本的改动是正确的，可以直接对所有人生效
        </label>

        {error && <div className="banner error">{error}</div>}

        {result && (
          <div className="banner info">
            <b>发布成功</b>
            <br />
            技能：{result.name}
            <br />
            路径：<span className="mono">{result.repoPath}</span>
            <br />
            提交：<span className="mono">{result.commitSha.slice(0, 8)}</span>
            <br />
            提交信息：{result.commitMessage}
            {result.snapshotId && (
              <>
                <br />
                已保存发布前快照：<span className="mono">{result.snapshotId}</span>
              </>
            )}
          </div>
        )}

        <div>
          <button className="primary" onClick={submit} disabled={busy || !ready}>
            {busy && <span className="spinner" />}
            发布到 master
          </button>
        </div>
      </div>
    </div>
  );
}
