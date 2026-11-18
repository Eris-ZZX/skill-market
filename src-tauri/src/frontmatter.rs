//! SKILL.md 解析
//!
//! 只依赖 `serde_yaml`，容错优先：
//! 实测本机有技能**连 frontmatter 都没有**（`abnormal-hours-approver`），
//! 解析必须能处理这种情况而不是崩掉。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 从 SKILL.md 提取的元信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    /// 技能标识（frontmatter 的 `name`，缺省时回退到目录名）
    pub name: String,
    /// 描述（四家工具唯一的共同必填项）
    pub description: String,
    /// 版本号 —— **仅用于展示，不作为更新判据**
    pub version: Option<String>,
    /// 细筛选用标签
    pub tags: Vec<String>,
    /// 粗分组（对应仓库目录）
    pub category: Option<String>,
    /// 作者列表
    pub authors: Vec<String>,
}

impl SkillMeta {
    /// 从文件路径解析；读不到文件时返回 Err
    pub fn from_path(skill_md: &Path) -> anyhow::Result<SkillMeta> {
        let raw = std::fs::read_to_string(skill_md)
            .map_err(|e| anyhow::anyhow!("读取 {} 失败: {e}", skill_md.display()))?;
        Ok(Self::from_str_content(&raw, None))
    }

    /// 从目录解析（自动补默认名 = 目录名）
    pub fn from_dir(dir: &Path) -> anyhow::Result<SkillMeta> {
        let fallback = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let md = dir.join("SKILL.md");
        if !md.is_file() {
            anyhow::bail!("{} 下没有 SKILL.md", dir.display());
        }
        let raw = std::fs::read_to_string(&md)?;
        Ok(Self::from_str_content(&raw, Some(&fallback)))
    }

    /// 解析内容。`fallback_name` 在 frontmatter 缺 `name` 时使用。
    pub fn from_str_content(raw: &str, fallback_name: Option<&str>) -> SkillMeta {
        let mut meta = SkillMeta::default();

        let text = raw.trim_start_matches('\u{feff}');
        if !text.starts_with("---") {
            // 无 frontmatter：全部退化为默认值
            meta.name = fallback_name.unwrap_or_default().to_string();
            return meta;
        }

        let after_open = &text[3..];
        let Some(close_rel) = after_open.find("\n---") else {
            meta.name = fallback_name.unwrap_or_default().to_string();
            return meta;
        };

        let fm = &after_open[..close_rel];

        // 解析失败不致命：用默认值继续
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(fm) {
            if let Some(map) = value.as_mapping() {
                meta.name = yaml_str(map, "name").unwrap_or_default();
                meta.description = yaml_str(map, "description").unwrap_or_default();
                meta.version = yaml_str(map, "version");

                // 我们自己的元信息放在 metadata.skillmarket 下（生态惯例）
                if let Some(sm) = map
                    .get(serde_yaml::Value::String("metadata".into()))
                    .and_then(|m| m.get("skillmarket"))
                    .and_then(|v| v.as_mapping())
                {
                    if meta.category.is_none() {
                        meta.category = yaml_str(sm, "category");
                    }
                    meta.tags = yaml_str_list(sm, "tags");
                    meta.authors = yaml_str_list(sm, "authors");
                }

                if meta.tags.is_empty() {
                    meta.tags = yaml_str_list(map, "tags");
                }
            }
        }

        if meta.name.is_empty() {
            meta.name = fallback_name.unwrap_or_default().to_string();
        }
        meta
    }

    /// 名称是否合法（小写字母、数字、连字符）
    ///
    /// 四家工具都要求技能名可做目录名且唯一，这是最安全的白名单。
    pub fn is_valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-')
    }
}

fn yaml_str(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    map.get(serde_yaml::Value::String(key.into()))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn yaml_str_list(map: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    match map.get(serde_yaml::Value::String(key.into())) {
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(serde_yaml::Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_frontmatter() {
        let raw = "---\nname: sales-report\ndescription: 生成销售周报\nversion: 1.2.0\n---\n# Body\n";
        let m = SkillMeta::from_str_content(raw, None);
        assert_eq!(m.name, "sales-report");
        assert_eq!(m.description, "生成销售周报");
        assert_eq!(m.version.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn parses_metadata_skillmarket_block() {
        let raw = r#"---
name: demo
description: d
metadata:
  skillmarket:
    category: 销售
    tags: [报表, 周报]
    authors: ["张三"]
---
body
"#;
        let m = SkillMeta::from_str_content(raw, None);
        assert_eq!(m.category.as_deref(), Some("销售"));
        assert_eq!(m.tags, vec!["报表", "周报"]);
        assert_eq!(m.authors, vec!["张三"]);
    }

    #[test]
    fn falls_back_to_dir_name_without_frontmatter() {
        let raw = "# 只有正文，没有 frontmatter\n";
        let m = SkillMeta::from_str_content(raw, Some("abnormal-hours-approver"));
        assert_eq!(m.name, "abnormal-hours-approver");
    }

    #[test]
    fn malformed_yaml_does_not_panic() {
        let raw = "---\nname: [unclosed\n---\nbody\n";
        let m = SkillMeta::from_str_content(raw, Some("fallback"));
        assert_eq!(m.name, "fallback");
    }

    #[test]
    fn validates_names() {
        assert!(SkillMeta::is_valid_name("sales-report"));
        assert!(SkillMeta::is_valid_name("abc123"));
        assert!(!SkillMeta::is_valid_name("Sales Report"));
        assert!(!SkillMeta::is_valid_name("sales_report"));
        assert!(!SkillMeta::is_valid_name("-leading"));
        assert!(!SkillMeta::is_valid_name(""));
    }
}
