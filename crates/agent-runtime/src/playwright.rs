use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptLanguage {
    TypeScript,
    Python,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedAction {
    pub tool: String,
    pub url: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub text: Option<String>,
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub submit: Option<bool>,
    pub selector_fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaywrightScript {
    pub language: ScriptLanguage,
    pub title: String,
    pub code: String,
    pub step_count: usize,
}

impl RecordedAction {
    pub fn navigate(url: impl Into<String>) -> Self {
        Self {
            tool: "navigate".into(),
            url: Some(url.into()),
            role: None,
            name: None,
            text: None,
            value: None,
            checked: None,
            submit: None,
            selector_fallback: None,
        }
    }

    pub fn click(role: Option<&str>, name: Option<&str>) -> Self {
        Self {
            tool: "click".into(),
            url: None,
            role: role.map(Into::into),
            name: name.map(Into::into),
            text: None,
            value: None,
            checked: None,
            submit: None,
            selector_fallback: None,
        }
    }

    pub fn fill(role: Option<&str>, name: Option<&str>, text: impl Into<String>, submit: bool) -> Self {
        Self {
            tool: "type".into(),
            url: None,
            role: role.map(Into::into),
            name: name.map(Into::into),
            text: Some(text.into()),
            value: None,
            checked: None,
            submit: Some(submit),
            selector_fallback: None,
        }
    }

    pub fn select(role: Option<&str>, name: Option<&str>, value: impl Into<String>) -> Self {
        Self {
            tool: "select".into(),
            url: None,
            role: role.map(Into::into),
            name: name.map(Into::into),
            text: None,
            value: Some(value.into()),
            checked: None,
            submit: None,
            selector_fallback: None,
        }
    }

    pub fn check(role: Option<&str>, name: Option<&str>, checked: bool) -> Self {
        Self {
            tool: "check".into(),
            url: None,
            role: role.map(Into::into),
            name: name.map(Into::into),
            text: None,
            value: None,
            checked: Some(checked),
            submit: None,
            selector_fallback: None,
        }
    }
}

/// Generate a resilient Playwright test script from a sequence of recorded browser actions (ADR-005, D-2).
pub fn generate_playwright_script(
    title: &str,
    actions: &[RecordedAction],
    language: ScriptLanguage,
) -> PlaywrightScript {
    let sanitized_title = if title.trim().is_empty() {
        "Automated Browser Workflow".to_string()
    } else {
        title.trim().to_string()
    };

    let code = match language {
        ScriptLanguage::TypeScript => generate_typescript(&sanitized_title, actions),
        ScriptLanguage::Python => generate_python(&sanitized_title, actions),
    };

    PlaywrightScript {
        language,
        title: sanitized_title,
        code,
        step_count: actions.len(),
    }
}

fn generate_typescript(title: &str, actions: &[RecordedAction]) -> String {
    let mut lines = Vec::new();
    lines.push("import { test, expect } from '@playwright/test';".to_string());
    lines.push(String::new());
    lines.push(format!("test('{}', async ({{ page }}) => {{", escape_str(title)));

    if actions.is_empty() {
        lines.push("  // No actions recorded in this session".to_string());
    }

    for (idx, action) in actions.iter().enumerate() {
        let step_num = idx + 1;
        match action.tool.as_str() {
            "navigate" => {
                if let Some(ref url) = action.url {
                    lines.push(format!("  // Step {step_num}: Navigate to {url}"));
                    lines.push(format!("  await page.goto('{}');", escape_str(url)));
                }
            }
            "click" => {
                let locator = build_ts_locator(action);
                lines.push(format!("  // Step {step_num}: Click"));
                lines.push(format!("  await {locator}.click();"));
            }
            "type" => {
                let locator = build_ts_locator(action);
                let text = action.text.as_deref().unwrap_or("");
                lines.push(format!("  // Step {step_num}: Fill text"));
                lines.push(format!("  await {locator}.fill('{}');", escape_str(text)));
                if action.submit.unwrap_or(false) {
                    lines.push("  await page.keyboard.press('Enter');".to_string());
                }
            }
            "select" => {
                let locator = build_ts_locator(action);
                let val = action.value.as_deref().unwrap_or("");
                lines.push(format!("  // Step {step_num}: Select option"));
                lines.push(format!("  await {locator}.selectOption('{}');", escape_str(val)));
            }
            "check" => {
                let locator = build_ts_locator(action);
                let is_checked = action.checked.unwrap_or(true);
                lines.push(format!("  // Step {step_num}: {}", if is_checked { "Check" } else { "Uncheck" }));
                if is_checked {
                    lines.push(format!("  await {locator}.check();"));
                } else {
                    lines.push(format!("  await {locator}.uncheck();"));
                }
            }
            _ => {
                lines.push(format!("  // Step {step_num}: Unhandled action '{}'", action.tool));
            }
        }
        lines.push(String::new());
    }

    lines.push("});".to_string());
    lines.join("\n")
}

fn generate_python(title: &str, actions: &[RecordedAction]) -> String {
    let mut lines = Vec::new();
    lines.push("import asyncio".to_string());
    lines.push("from playwright.async_api import async_playwright".to_string());
    lines.push(String::new());
    lines.push(format!("# Scenario: {}", title));
    lines.push("async def main():".to_string());
    lines.push("    async with async_playwright() as p:".to_string());
    lines.push("        browser = await p.chromium.launch(headless=False)".to_string());
    lines.push("        page = await browser.new_page()".to_string());
    lines.push(String::new());

    if actions.is_empty() {
        lines.push("        # No actions recorded in this session".to_string());
    }

    for (idx, action) in actions.iter().enumerate() {
        let step_num = idx + 1;
        match action.tool.as_str() {
            "navigate" => {
                if let Some(ref url) = action.url {
                    lines.push(format!("        # Step {step_num}: Navigate to {url}"));
                    lines.push(format!("        await page.goto(\"{}\")", escape_str(url)));
                }
            }
            "click" => {
                let locator = build_py_locator(action);
                lines.push(format!("        # Step {step_num}: Click"));
                lines.push(format!("        await {locator}.click()"));
            }
            "type" => {
                let locator = build_py_locator(action);
                let text = action.text.as_deref().unwrap_or("");
                lines.push(format!("        # Step {step_num}: Fill text"));
                lines.push(format!("        await {locator}.fill(\"{}\")", escape_str(text)));
                if action.submit.unwrap_or(false) {
                    lines.push("        await page.keyboard.press(\"Enter\")".to_string());
                }
            }
            "select" => {
                let locator = build_py_locator(action);
                let val = action.value.as_deref().unwrap_or("");
                lines.push(format!("        # Step {step_num}: Select option"));
                lines.push(format!("        await {locator}.select_option(\"{}\")", escape_str(val)));
            }
            "check" => {
                let locator = build_py_locator(action);
                let is_checked = action.checked.unwrap_or(true);
                lines.push(format!("        # Step {step_num}: {}", if is_checked { "Check" } else { "Uncheck" }));
                if is_checked {
                    lines.push(format!("        await {locator}.check()"));
                } else {
                    lines.push(format!("        await {locator}.uncheck()"));
                }
            }
            _ => {
                lines.push(format!("        # Step {step_num}: Unhandled action '{}'", action.tool));
            }
        }
        lines.push(String::new());
    }

    lines.push("        await browser.close()".to_string());
    lines.push(String::new());
    lines.push("if __name__ == '__main__':".to_string());
    lines.push("    asyncio.run(main())".to_string());

    lines.join("\n")
}

fn build_ts_locator(action: &RecordedAction) -> String {
    if let Some(ref role) = action.role {
        if let Some(ref name) = action.name {
            return format!("page.getByRole('{}', {{ name: '{}' }})", escape_str(role), escape_str(name));
        }
        return format!("page.getByRole('{}')", escape_str(role));
    }
    if let Some(ref name) = action.name {
        return format!("page.getByText('{}')", escape_str(name));
    }
    if let Some(ref sel) = action.selector_fallback {
        return format!("page.locator('{}')", escape_str(sel));
    }
    "page.locator('body')".to_string()
}

fn build_py_locator(action: &RecordedAction) -> String {
    if let Some(ref role) = action.role {
        if let Some(ref name) = action.name {
            return format!("page.get_by_role(\"{}\", name=\"{}\")", escape_str(role), escape_str(name));
        }
        return format!("page.get_by_role(\"{}\")", escape_str(role));
    }
    if let Some(ref name) = action.name {
        return format!("page.get_by_text(\"{}\")", escape_str(name));
    }
    if let Some(ref sel) = action.selector_fallback {
        return format!("page.locator(\"{}\")", escape_str(sel));
    }
    "page.locator(\"body\")".to_string()
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'").replace('"', "\\\"").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_typescript_test_with_resilient_roles() {
        let actions = vec![
            RecordedAction::navigate("https://shop.localhost:3000/products"),
            RecordedAction::fill(Some("textbox"), Some("Search"), "running shoes", true),
            RecordedAction::click(Some("link"), Some("Nike Pegasus")),
            RecordedAction::click(Some("button"), Some("Add to Cart")),
            RecordedAction::check(Some("checkbox"), Some("Gift Wrap"), true),
        ];

        let script = generate_playwright_script("Shop Workflow", &actions, ScriptLanguage::TypeScript);
        assert_eq!(script.step_count, 5);
        assert_eq!(script.language, ScriptLanguage::TypeScript);
        assert!(script.code.contains("import { test, expect } from '@playwright/test';"));
        assert!(script.code.contains("test('Shop Workflow', async ({ page }) => {"));
        assert!(script.code.contains("await page.goto('https://shop.localhost:3000/products');"));
        assert!(script.code.contains("await page.getByRole('textbox', { name: 'Search' }).fill('running shoes');"));
        assert!(script.code.contains("await page.keyboard.press('Enter');"));
        assert!(script.code.contains("await page.getByRole('link', { name: 'Nike Pegasus' }).click();"));
        assert!(script.code.contains("await page.getByRole('button', { name: 'Add to Cart' }).click();"));
        assert!(script.code.contains("await page.getByRole('checkbox', { name: 'Gift Wrap' }).check();"));
    }

    #[test]
    fn generates_python_script_with_resilient_roles() {
        let actions = vec![
            RecordedAction::navigate("https://example.com/login"),
            RecordedAction::fill(Some("textbox"), Some("Email"), "user@example.com", false),
            RecordedAction::click(Some("button"), Some("Sign In")),
        ];

        let script = generate_playwright_script("Login Scenario", &actions, ScriptLanguage::Python);
        assert_eq!(script.step_count, 3);
        assert_eq!(script.language, ScriptLanguage::Python);
        assert!(script.code.contains("from playwright.async_api import async_playwright"));
        assert!(script.code.contains("await page.goto(\"https://example.com/login\")"));
        assert!(script.code.contains("await page.get_by_role(\"textbox\", name=\"Email\").fill(\"user@example.com\")"));
        assert!(script.code.contains("await page.get_by_role(\"button\", name=\"Sign In\").click()"));
    }
}
