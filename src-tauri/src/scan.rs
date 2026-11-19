//! 扫描各工具的技能目录
//!
//! 「我的技能」的数据来源必须是磁盘而不是 `state.json`：技能可能是别的方式装进来的
//! （cc-switch 软链、手工复制、工具自带同步），只有扫目录才看得见。
//! `state.json` 退居为「来源与安装时指纹」的补充。
//!
//! 扫描全程只读，绝不创建目录（R1）。

use crate::frontmatter::SkillMeta;
use crate::tools::{self, ToolKind};
use serde::Serialize;
use std::path::PathBuf;

/// 扫描到的一个技能
///
/// 这不是安装记录：它只说明「这个目录里有一个技能」，不代表本客户端装过。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedSkill {
    /// frontmatter 里的 name，缺失时回退目录名
    pub name: String,
    pub dir_name: String,
    /// 工具 key
    pub tool: String,
    /// 目录绝对路径
    pub target_path: String,
    pub description: String,
    pub version: Option<String>,
    pub tags: Vec<String>,
    pub category: Option<String>,
    /// 软链或 Windows 重解析点
    pub is_link: bool,
    pub link_target: Option<String>,
}

/// 扫描全部六个工具
pub fn scan_all() -> Vec<ScannedSkill> {
    let roots: Vec<(String, PathBuf)> = ToolKind::ALL
        .iter()
        .filter_map(|t| t.skills_dir().map(|d| (t.key().to_string(), d)))
        .collect();
    scan_dirs(&roots)
}

/// 扫描给定的 (工具 key, skills 目录) 列表
///
/// 接受显式目录是为了能在测试里用 tempdir 隔离，不去碰真实用户目录。
pub fn scan_dirs(roots: &[(String, PathBuf)]) -> Vec<ScannedSkill> {
    let mut out = Vec::new();
    for (tool, dir) in roots {
        // 目录不存在 = 该工具没装，静默跳过
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            // is_dir 会跟随软链，正是我们要的：cc-switch 的软链技能也该被看见
            if !path.is_dir() {
                continue;
            }
            // 只认直接子目录下的 SKILL.md，不递归——否则技能内部的
            // scripts/ 之类会被误认成技能
            if !path.join("SKILL.md").is_file() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().to_string();
            // 解析失败不能中断整体扫描，退化为只有目录名的条目
            let meta = SkillMeta::from_dir(&path).unwrap_or_default();
            let is_link = tools::is_link(&path);
            let link_target = if is_link {
                std::fs::read_link(&path)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            } else {
                None
            };

            out.push(ScannedSkill {
                name: if meta.name.is_empty() {
                    dir_name.clone()
                } else {
                    meta.name
                },
                dir_name,
                tool: tool.clone(),
                target_path: path.to_string_lossy().to_string(),
                description: meta.description,
                version: meta.version,
                tags: meta.tags,
                category: meta.category,
                is_link,
                link_target,
            });
        }
    }
    // 顺序稳定，便于前端与测试比对
    out.sort_by(|a, b| a.tool.cmp(&b.tool).then_with(|| a.dir_name.cmp(&b.dir_name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    #[test]
    fn 有_skill_md_的子目录被识别_无的被跳过() {
        let root = tempfile::tempdir().unwrap();
        let skills = root.path().join("skills");
        write(&skills, "good/SKILL.md", "---\nname: good\ndescription: 好技能\n---\nbody\n");
        // 没有 SKILL.md 的目录
        write(&skills, "not-a-skill/readme.txt", "x");
        // 只有深层才有 SKILL.md，不应被当成直接子目录技能
        write(&skills, "nested/inner/SKILL.md", "---\nname: inner\n---\n");

        let got = scan_dirs(&[("cursor".to_string(), skills)]);
        let names: Vec<_> = got.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["good"], "只认直接子目录下的 SKILL.md");
        assert_eq!(got[0].description, "好技能");
        assert_eq!(got[0].tool, "cursor");
    }

    #[test]
    fn frontmatter_缺失不崩且回退目录名() {
        let root = tempfile::tempdir().unwrap();
        let skills = root.path().join("skills");
        write(&skills, "abnormal-hours-approver/SKILL.md", "# 没有 frontmatter\n");

        let got = scan_dirs(&[("claude".to_string(), skills)]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "abnormal-hours-approver");
        assert_eq!(got[0].description, "");
    }

    #[test]
    fn 工具目录不存在时静默跳过() {
        let root = tempfile::tempdir().unwrap();
        let got = scan_dirs(&[("codex".to_string(), root.path().join("nope"))]);
        assert!(got.is_empty());
    }

    #[test]
    fn 扫描不创建任何目录() {
        let root = tempfile::tempdir().unwrap();
        let skills = root.path().join("skills");
        write(&skills, "a/SKILL.md", "---\nname: a\n---\n");

        let before = list_all(&root.path().to_path_buf());
        let _ = scan_dirs(&[("agents".to_string(), skills.clone())]);
        // 指向一个根本不存在的目录，也不该被创建
        let _ = scan_dirs(&[("agents".to_string(), root.path().join("missing"))]);
        let after = list_all(&root.path().to_path_buf());

        assert_eq!(before, after, "扫描必须纯只读");
        assert!(!root.path().join("missing").exists());
    }

    fn list_all(root: &PathBuf) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(cur: &Path, out: &mut Vec<String>) {
            if let Ok(rd) = fs::read_dir(cur) {
                for e in rd.flatten() {
                    out.push(e.path().to_string_lossy().to_string());
                    if e.path().is_dir() {
                        walk(&e.path(), out);
                    }
                }
            }
        }
        walk(root, &mut out);
        out.sort();
        out
    }

    #[test]
    fn 软链技能被识别且标出目标() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real-skill");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("SKILL.md"), "---\nname: linked\n---\nbody\n").unwrap();

        let skills = root.path().join("skills");
        fs::create_dir_all(&skills).unwrap();

        // Windows 上创建软链需要开发者模式或管理员权限；
        // 拿不到权限就跳过，不假装通过。
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&real, skills.join("linked")).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&real, skills.join("linked")).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;
        if !made {
            eprintln!("跳过：当前环境无权创建目录软链");
            return;
        }

        let got = scan_dirs(&[("claude".to_string(), skills)]);
        assert_eq!(got.len(), 1);
        assert!(got[0].is_link, "软链必须被标记");
        assert!(got[0].link_target.is_some(), "应读出链接目标");
        assert_eq!(got[0].name, "linked", "软链里的 SKILL.md 应可读");
    }
}
