import { useEffect, useState } from "react";
import { api, AppInfo, AppSettings, ToolDetection } from "../api";

interface Props {
  tools: ToolDetection[];
  settings: AppSettings;
  onSaved: (s: AppSettings) => void;
  onToolsChanged: (t: ToolDetection[]) => void;
}

export default function SettingsPage({
  tools,
  settings,
  onSaved,
  onToolsChanged,
}: Props) {
  const [form, setForm] = useState<AppSettings>(settings);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<
    { ok: boolean; msg: string } | null
  >(null);

  useEffect(() => {
    api.appInfo().then(setInfo).catch(() => {});
  }, []);

  const dirty = JSON.stringify(form) !== JSON.stringify(settings);

  const set = <K extends keyof AppSettings>(k: K, v: AppSettings[K]) => {
    setForm((f) => ({ ...f, [k]: v }));
    setSaved(false);
    setTestResult(null);
  };

  const save = async () => {
    setSaving(true);
    try {
      await api.saveSettings(form);
      onSaved(form);
      setSaved(true);
    } catch (e) {
      alert(String(e));
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      // 先保存再测，保证后端读到的是最新配置
      await api.saveSettings(form);
      onSaved(form);
      const list = await api.refreshIndex();
      setTestResult({
        ok: true,
        msg: `连接成功，仓库里有 ${list.length} 个技能`,
      });
    } catch (e) {
      setTestResult({ ok: false, msg: String(e) });
    } finally {
      setTesting(false);
    }
  };

  const redetect = async () => {
    const t = await api.detectTools();
    onToolsChanged(t);
  };

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">设置</h1>
      </div>

      <div className="form">
        <div className="field">
          <label>GitLab 地址</label>
          <input
            placeholder="https://gitlab.company.com"
            value={form.gitlabBaseUrl}
            onChange={(e) => set("gitlabBaseUrl", e.target.value)}
          />
          <div className="hint">
            内网 GitLab 实例根地址。不要带末尾斜杠，不要带 /api/v4。
          </div>
        </div>

        <div className="field">
          <label>项目路径</label>
          <input
            placeholder="group/subgroup/skills-repo"
            value={form.gitlabProjectPath}
            onChange={(e) => set("gitlabProjectPath", e.target.value)}
          />
          <div className="hint">
            仓库在 GitLab 上的完整路径。支持子组，如 group/sub/repo。
          </div>
        </div>

        <div className="field">
          <label>分支</label>
          <input
            value={form.gitlabBranch}
            onChange={(e) => set("gitlabBranch", e.target.value)}
          />
        </div>

        <div className="field">
          <label>访问令牌（可选）</label>
          <input
            type="password"
            placeholder="仓库公开可读时留空"
            value={form.gitlabToken ?? ""}
            onChange={(e) => set("gitlabToken", e.target.value || null)}
          />
          <div className="hint">
            仓库设为 Internal / Public 时不需要；私有仓库需填只读 Token。
            令牌只保存在本机 state.json，不会上传。
          </div>
        </div>

        <div className="field">
          <label>发布人署名</label>
          <input
            placeholder="姓名/工号，如 张三/1234"
            value={form.publisherIdentity}
            onChange={(e) => set("publisherIdentity", e.target.value)}
          />
          <div className="hint">
            发布技能时写进 commit message 与 SKILL.md 的 authors。
            所有客户端共用一个 Token，GitLab 只能看到机器人账号，
            因此这是识别「谁改的」的唯一依据。
          </div>
        </div>

        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button className="primary" onClick={save} disabled={saving || !dirty}>
            {saving && <span className="spinner" />}
            保存
          </button>
          <button onClick={test} disabled={testing}>
            {testing && <span className="spinner" />}
            测试连接
          </button>
          {saved && !dirty && <span className="page-sub">已保存</span>}
          {dirty && <span className="page-sub">有未保存的修改</span>}
        </div>

        {testResult && (
          <div className={`banner ${testResult.ok ? "info" : "error"}`}>
            {testResult.msg}
          </div>
        )}

        <div className="field" style={{ marginTop: 6 }}>
          <label
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
            }}
          >
            <span>已检测到的 AI 工具</span>
            <button className="ghost sm" onClick={redetect}>
              重新探测
            </button>
          </label>
          <div className="tool-list">
            {tools.map((t) => (
              <div className="tool-row" key={t.key}>
                <span className={`dot ${t.installed ? "on" : "off"}`} />
                <span style={{ minWidth: 112 }}>{t.displayName}</span>
                <span className="page-sub">
                  {t.installed
                    ? t.skillsDirExists
                      ? `${t.installedSkillCount} 个技能`
                      : "缺 skills 目录"
                    : "未安装"}
                </span>
                <span
                  className="mono"
                  style={{
                    marginLeft: "auto",
                    maxWidth: 240,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                  title={t.rootDir ?? ""}
                >
                  {t.rootDir ?? "—"}
                </span>
              </div>
            ))}
          </div>
          <div className="hint">
            探测是只读的：只有工具目录已存在才会出现在安装选项里。
            本客户端不会创建工具根目录。
          </div>
        </div>

        {info && (
          <>
            <hr
              style={{
                border: "none",
                borderTop: "1px solid var(--border)",
                margin: "4px 0",
                width: "100%",
              }}
            />
            <div className="field">
              <label>数据目录</label>
              <div className="mono break">{info.dataDir}</div>
              <div className="hint">
                state.json 存安装记录，index.json 缓存技能索引。
              </div>
            </div>
            <div className="page-sub" style={{ lineHeight: 1.9 }}>
              Skill Market v{info.version}
              <br />
              junction 支持：
              {info.junctionSupported ? "可用" : "不可用（将使用复制模式）"}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
