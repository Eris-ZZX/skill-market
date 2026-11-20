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

/// 一个落盘位置
///
/// 带上 `is_link` / `link_target`：同一个技能在不同工具下落盘方式可能不同
/// （一处是软链、一处是实体复制），聚合后不能再共用外层那两个字段。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPlacement {
    pub tool: String,
    pub target_path: String,
    pub is_link: bool,
    pub link_target: Option<String>,
}

/// 扫描到的一个技能
///
/// 这不是安装记录：它只说明「这些目录里有一个技能」，不代表本客户端装过。
///
/// 聚合键是**目录名**而不是 frontmatter 的 `name`：目录名在文件系统里唯一，
/// 而 `name` 可能被两个内容不同的技能重名写。按 `name` 聚合会把它们错并成一行。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedSkill {
    /// frontmatter 里的 name，缺失时回退目录名
    pub name: String,
    pub dir_name: String,
    /// 工具 key（首个落盘；完整列表见 `placements`）
    pub tool: String,
    /// 目录绝对路径（首个落盘；完整列表见 `placements`）
    pub target_path: String,
    /// 在哪些工具的目录下发现
    pub found_in: Vec<String>,
    /// 每个落盘的完整信息，供后续操作定位
    pub placements: Vec<SkillPlacement>,
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
///
/// 返回前按 `dir_name` 聚合成一个技能一条：同一技能装在多个工具下
/// （cc-switch 的软链就是把同一份目录链到多个工具）会合成一条并记录
/// 全部落盘位置；目录名不同的技能即使 frontmatter 同名也不会合并。
pub fn scan_dirs(roots: &[(String, PathBuf)]) -> Vec<ScannedSkill> {
    let mut raw = scan_each_placement(roots);

    // 顺序稳定，便于前端与测试比对
    raw.sort_by(|a, b| a.tool.cmp(&b.tool).then_with(|| a.dir_name.cmp(&b.dir_name)));

    let mut order: Vec<String> = Vec::new();
    let mut by_dir: std::collections::HashMap<String, ScannedSkill> = Default::default();

    for s in raw {
        let placement = SkillPlacement {
            tool: s.tool.clone(),
            target_path: s.target_path.clone(),
            is_link: s.is_link,
            link_target: s.link_target.clone(),
        };
        match by_dir.get_mut(&s.dir_name) {
            Some(existing) => {
                if !existing.found_in.contains(&s.tool) {
                    existing.found_in.push(s.tool.clone());
                }
                existing.placements.push(placement);
                // 首个落盘已经定了 tool/target_path，这里只补空字段
                if existing.description.is_empty() {
                    existing.description = s.description;
                }
                if existing.version.is_none() {
                    existing.version = s.version;
                }
            }
            None => {
                order.push(s.dir_name.clone());
                by_dir.insert(
                    s.dir_name.clone(),
                    ScannedSkill {
                        name: s.name,
                        dir_name: s.dir_name,
                        tool: s.tool.clone(),
                        target_path: s.target_path,
                        found_in: vec![s.tool],
                        placements: vec![placement],
                        description: s.description,
                        version: s.version,
                        tags: s.tags,
                        category: s.category,
                        is_link: s.is_link,
                        link_target: s.link_target,
                    },
                );
            }
        }
    }

    order
        .into_iter()
        .filter_map(|d| by_dir.remove(&d))
        .collect()
}

/// 逐「技能 × 工具」扫描，不做聚合
fn scan_each_placement(roots: &[(String, PathBuf)]) -> Vec<ScannedSkill> {
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
                found_in: Vec::new(),
                placements: Vec::new(),
                description: meta.description,
                version: meta.version,
                tags: meta.tags,
                category: meta.category,
                is_link,
                link_target,
            });
        }
    }
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

    #[test]
    fn 同一技能装在三个工具下聚合成一条() {
        let root = tempfile::tempdir().unwrap();
        let mut roots = Vec::new();
        for tool in ["claude", "codex", "agents"] {
            let skills = root.path().join(tool).join("skills");
            write(&skills, "grill-me/SKILL.md", "---\nname: grill-me\ndescription: d\n---\n");
            roots.push((tool.to_string(), skills));
        }

        let got = scan_dirs(&roots);
        assert_eq!(got.len(), 1, "同一目录名的技能应聚合成一条");
        assert_eq!(got[0].found_in.len(), 3, "三个工具都应被记录");
        assert_eq!(got[0].placements.len(), 3, "三处落盘都应保留");
        // 排序后工具顺序稳定
        assert_eq!(got[0].found_in, vec!["agents", "claude", "codex"]);
    }

    #[test]
    fn 同名不同目录不合并() {
        // 两个目录内容完全不同，只是 frontmatter 都写了同一个 name。
        // 按 name 聚合会把它们错并成一条，用户会以为只有一个技能。
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a").join("skills");
        let b = root.path().join("b").join("skills");
        write(&a, "grill-me/SKILL.md", "---\nname: grill-me\ndescription: 甲的版本\n---\n");
        write(&b, "grill-me-copy/SKILL.md", "---\nname: grill-me\ndescription: 乙的版本\n---\n");

        let got = scan_dirs(&[
            ("claude".to_string(), a),
            ("codex".to_string(), b),
        ]);

        assert_eq!(got.len(), 2, "目录名不同就该是两条，哪怕 name 相同");
        let names: Vec<&str> = got.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["grill-me", "grill-me"], "两条的 name 相同");
        let dirs: Vec<&str> = got.iter().map(|s| s.dir_name.as_str()).collect();
        assert_ne!(dirs[0], dirs[1], "但目录名不同");
        // 两条各自只有一处落盘
        assert!(got.iter().all(|s| s.placements.len() == 1));
        assert!(got.iter().all(|s| s.found_in.len() == 1));
    }

    #[test]
    fn 没有_skill_md_的目录被跳过且不计入聚合() {
        let root = tempfile::tempdir().unwrap();
        let skills = root.path().join("a").join("skills");
        write(&skills, "real/SKILL.md", "---\nname: real\n---\n");
        // 同名的空目录：不该因为它存在就把 real 变成两条
        fs::create_dir_all(skills.join("empty")).unwrap();

        let got = scan_dirs(&[("claude".to_string(), skills)]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].dir_name, "real");
    }

    #[test]
    fn found_in_与_placements_一致() {
        let root = tempfile::tempdir().unwrap();
        let mut roots = Vec::new();
        for tool in ["claude", "codex"] {
            let skills = root.path().join(tool).join("skills");
            write(&skills, "x/SKILL.md", "---\nname: x\n---\n");
            roots.push((tool.to_string(), skills));
        }
        let got = scan_dirs(&roots);
        assert_eq!(got.len(), 1);
        let from_found: Vec<&str> = got[0].found_in.iter().map(|s| s.as_str()).collect();
        let from_place: Vec<&str> = got[0]
            .placements
            .iter()
            .map(|p| p.tool.as_str())
            .collect();
        assert_eq!(from_found, from_place, "两个列表必须一一对应");
    }

    #[test]
    fn 聚合不丢技能() {
        // 3 个工具 × 各 2 个不同目录名的技能 = 6 条原始记录，
        // 聚合后应是 2 条（每个目录名一条），技能数不能少
        let root = tempfile::tempdir().unwrap();
        let mut roots = Vec::new();
        for tool in ["claude", "codex", "agents"] {
            let skills = root.path().join(tool).join("skills");
            write(&skills, "skill-a/SKILL.md", "---\nname: a\n---\n");
            write(&skills, "skill-b/SKILL.md", "---\nname: b\n---\n");
            roots.push((tool.to_string(), skills));
        }

        let got = scan_dirs(&roots);
        assert_eq!(got.len(), 2, "两个不同目录名应聚成两条");
        let total_placements: usize = got.iter().map(|s| s.placements.len()).sum();
        assert_eq!(total_placements, 6, "6 个落盘一个都不能丢");
        let mut dirs: Vec<&str> = got.iter().map(|s| s.dir_name.as_str()).collect();
        dirs.sort();
        assert_eq!(dirs, vec!["skill-a", "skill-b"]);
    }
}
