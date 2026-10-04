//! Skills and automation scenarios (ADR-008).
//!
//! Declarative skills configure the agent with reusable instructions,
//! pre-configured task scopes, recommended tools, and parameter schemas.

use crate::TaskScope;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillParameter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub prompt_template: String,
    pub scope: TaskScope,
    #[serde(default)]
    pub parameters: Vec<SkillParameter>,
}

impl SkillDefinition {
    /// Renders the prompt template by substituting `{{param}}` placeholders.
    pub fn render_prompt(&self, params: &std::collections::HashMap<String, String>) -> String {
        let mut rendered = self.prompt_template.clone();
        for (k, v) in params {
            rendered = rendered.replace(&format!("{{{{{k}}}}}"), v);
        }
        rendered
    }
}

/// Built-in library of skills for common developer and browsing tasks.
pub fn builtin_skills() -> Vec<SkillDefinition> {
    vec![
        SkillDefinition {
            id: "summarize_doc".into(),
            name: "Summarize Documentation".into(),
            description: "Extract key concepts, APIs, and code examples from technical documentation".into(),
            category: "research".into(),
            prompt_template: "Read and summarize {{doc_url}}. Highlight all core functions and configuration options.".into(),
            scope: TaskScope::new(
                vec!["https://*"],
                vec!["navigate", "extract", "read_more"],
            ),
            parameters: vec![
                SkillParameter {
                    name: "doc_url".into(),
                    description: "Target documentation URL".into(),
                    required: true,
                    default_value: None,
                },
            ],
        },
        SkillDefinition {
            id: "github_issue_checker".into(),
            name: "GitHub Issue Triage".into(),
            description: "Check open issues for a repository and extract bug reports".into(),
            category: "development".into(),
            prompt_template: "Inspect issues on https://github.com/{{repo}}/issues and summarize the top 3 open bug reports.".into(),
            scope: TaskScope::new(
                vec!["https://github.com"],
                vec!["navigate", "click", "extract"],
            ),
            parameters: vec![
                SkillParameter {
                    name: "repo".into(),
                    description: "Repository in owner/name format".into(),
                    required: true,
                    default_value: None,
                },
            ],
        },
        SkillDefinition {
            id: "price_comparison".into(),
            name: "Compare Product Prices".into(),
            description: "Find product price and in-stock availability across authorized sellers".into(),
            category: "shopping".into(),
            prompt_template: "Search for {{product_name}} on {{store_url}}, extract the price and availability without adding to cart.".into(),
            scope: TaskScope::new(
                vec!["https://*"],
                vec!["navigate", "type", "click", "extract"],
            ),
            parameters: vec![
                SkillParameter {
                    name: "product_name".into(),
                    description: "Name or model of product".into(),
                    required: true,
                    default_value: None,
                },
                SkillParameter {
                    name: "store_url".into(),
                    description: "Target store web address".into(),
                    required: true,
                    default_value: None,
                },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn skill_prompt_rendering() {
        let skills = builtin_skills();
        let doc_skill = skills.iter().find(|s| s.id == "summarize_doc").unwrap();

        let mut params = HashMap::new();
        params.insert("doc_url".into(), "https://docs.rs/tokio".into());

        let rendered = doc_skill.render_prompt(&params);
        assert_eq!(
            rendered,
            "Read and summarize https://docs.rs/tokio. Highlight all core functions and configuration options."
        );
        assert!(doc_skill.scope.allows_tool("navigate"));
    }
}
