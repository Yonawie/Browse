//! CLI command for safety, phishing, and dark pattern inspection (Phase S8).

use page_intelligence::{detect_dark_patterns, inspect_url_phishing, PhishingSeverity};

pub async fn safety_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        eprintln!("usage: browse-desktop inspect-safety <url> [--text \"<optional page text>\"]");
        return Ok(());
    }

    let url = &args[0];
    let mut text = String::new();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--text" && i + 1 < args.len() {
            text = args[i + 1].clone();
            i += 2;
        } else {
            i += 1;
        }
    }

    let phishing = inspect_url_phishing(url);
    let dark_patterns = if !text.is_empty() {
        detect_dark_patterns(&text)
    } else {
        Vec::new()
    };

    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║              Browse Safety & Phishing Inspection              ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");
    println!("Target URL: {}", phishing.url);
    println!("Domain:     {}", phishing.domain);

    let severity_str = match phishing.severity {
        PhishingSeverity::Safe => "SAFE 🛡️",
        PhishingSeverity::Suspicious => "SUSPICIOUS ⚠",
        PhishingSeverity::Dangerous => "DANGEROUS 🚨",
    };
    println!("Verdict:    {severity_str}");

    println!("\n[Phishing / Homograph / Brand Heuristics]");
    if phishing.reasons.is_empty() {
        println!("  ✔ Clean: No homoglyphs, brand spoofing, or deceptive IP patterns.");
    } else {
        for reason in &phishing.reasons {
            println!("  ⚠ Finding: {reason}");
        }
    }

    if !text.is_empty() {
        println!("\n[Deceptive Web Design (Dark Patterns)]");
        if dark_patterns.is_empty() {
            println!("  ✔ Clean: No artificial scarcity, fake urgency, or recurring charge traps.");
        } else {
            for dp in &dark_patterns {
                println!("  ⚠ {}: \"{}\"", dp.title, dp.snippet);
                println!("    Explanation: {}", dp.explanation);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_safety_arguments() {
        let args = vec![
            "https://paypal.com".to_string(),
            "--text".to_string(),
            "Welcome to PayPal checkout".to_string(),
        ];
        assert!(safety_command(&args).await.is_ok());
    }
}
