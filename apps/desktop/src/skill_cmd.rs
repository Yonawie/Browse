//! `browse-desktop skill` — Skills CLI management (ADR-008).
//!
//! List, inspect, and run declarative skills from the command line.

use core_types::builtin_skills;
use std::collections::HashMap;

pub async fn skill_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.first().map(String::as_str).unwrap_or("list");

    match subcmd {
        "list" => {
            println!("Available Browse Skills (ADR-008):\n");
            for s in builtin_skills() {
                println!("• [{}] {} (category: {})", s.id, s.name, s.category);
                println!("  Description: {}", s.description);
                println!("  Allowed Tools: {}", s.scope.tools.join(", "));
                if !s.parameters.is_empty() {
                    let params = s.parameters.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
                    println!("  Parameters: {}", params);
                }
                println!();
            }
        }
        "render" => {
            if args.len() < 2 {
                eprintln!("usage: browse-desktop skill render <skill_id> [param=value ...]");
                return Ok(());
            }
            let skill_id = &args[1];
            let skills = builtin_skills();
            let Some(skill) = skills.iter().find(|s| s.id == *skill_id) else {
                eprintln!("Skill `{skill_id}` not found. Run `browse-desktop skill list`.");
                return Ok(());
            };

            let mut params_map = HashMap::new();
            for arg in &args[2..] {
                if let Some((k, v)) = arg.split_once('=') {
                    params_map.insert(k.to_string(), v.to_string());
                }
            }

            let prompt = skill.render_prompt(&params_map);
            println!("Skill: {}", skill.name);
            println!("Scope origins: {:?}", skill.scope.origins);
            println!("Scope tools: {:?}", skill.scope.tools);
            println!("\nRendered Prompt:\n{}", prompt);
        }
        _ => {
            eprintln!("usage: browse-desktop skill <list | render <id> [param=val ...]>");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_skill_list() {
        assert!(skill_command(&["list".to_string()]).await.is_ok());
    }

    #[tokio::test]
    async fn parses_skill_render() {
        assert!(skill_command(&[
            "render".to_string(),
            "github_issue_checker".to_string(),
            "repo=Yonawie/Browse".to_string()
        ]).await.is_ok());
    }
}
