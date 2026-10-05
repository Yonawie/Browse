use serde::{Deserialize, Serialize};

/// Console error or warning diagnostic input (D-1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsoleDiagnosticInput {
    pub message: String,
    pub level: String, // "error", "warning"
    pub source: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub stack_trace: Option<String>,
}

/// Network failure diagnostic input (D-1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkDiagnosticInput {
    pub url: String,
    pub method: String,
    pub status: u16,
    pub status_text: Option<String>,
    pub error_text: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
}

/// Categorization of common DevTools diagnostics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCategory {
    CorsPolicy,
    ContentSecurityPolicy,
    MixedContent,
    AuthenticationFailure,
    AuthorizationForbidden,
    NotFound,
    ServerCrash,
    NetworkTimeoutOrRefused,
    JavaScriptException,
    DeprecationWarning,
    GenericError,
}

/// Comprehensive, actionable explanation of a developer diagnostic (D-1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevToolsExplanation {
    pub category: DiagnosticCategory,
    pub title: String,
    pub summary: String,
    pub root_cause: String,
    pub suggested_fix: String,
    pub security_implications: Option<String>,
    pub redacted_sample: String,
}

/// Redact sensitive information (tokens, passwords, keys) from URL and text (ADR-007).
pub fn redact_secrets(input: &str) -> String {
    let mut out = String::new();
    let lower = input.to_lowercase();
    let mut cursor = 0;

    let sensitive_keys = ["token=", "key=", "secret=", "password=", "api_key=", "auth="];
    let bearer_prefixes = ["bearer ey", "bearer "];

    while cursor < input.len() {
        // Check sensitive query/param keys
        let mut matched_key_len = None;
        for key in &sensitive_keys {
            if lower[cursor..].starts_with(key) {
                matched_key_len = Some(key.len());
                break;
            }
        }

        if let Some(k_len) = matched_key_len {
            out.push_str(&input[cursor..cursor + k_len]);
            cursor += k_len;
            let val_len = input[cursor..]
                .find(|c: char| c == '&' || c == '#' || c == '"' || c == '\'' || c.is_whitespace())
                .unwrap_or(input.len() - cursor);
            if val_len > 0 {
                out.push_str("[REDACTED]");
                cursor += val_len;
            }
            continue;
        }

        // Check Bearer token prefixes
        let mut matched_bearer_len = None;
        for b in &bearer_prefixes {
            if lower[cursor..].starts_with(b) {
                matched_bearer_len = Some(b.len());
                break;
            }
        }

        if let Some(b_len) = matched_bearer_len {
            out.push_str(&input[cursor..cursor + b_len]);
            cursor += b_len;
            let val_len = input[cursor..]
                .find(|c: char| c == '"' || c == '\'' || c == ',' || c.is_whitespace())
                .unwrap_or(input.len() - cursor);
            if val_len > 0 {
                out.push_str("[REDACTED_TOKEN]");
                cursor += val_len;
            }
            continue;
        }

        if let Some(ch) = input[cursor..].chars().next() {
            out.push(ch);
            cursor += ch.len_utf8();
        } else {
            break;
        }
    }

    out
}

/// Redact sensitive HTTP headers.
pub fn redact_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(k, v)| {
            let lower = k.to_lowercase();
            if lower == "authorization" || lower == "cookie" || lower == "set-cookie" || lower == "x-api-key" {
                (k.clone(), "[REDACTED]".to_string())
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect()
}

/// Explain a console error or warning with actionable root cause and remediation (D-1).
pub fn explain_console_error(diag: &ConsoleDiagnosticInput) -> DevToolsExplanation {
    let msg_lower = diag.message.to_lowercase();
    let redacted_sample = redact_secrets(&diag.message);

    // 1. CORS Policy
    if msg_lower.contains("cors policy")
        || msg_lower.contains("access-control-allow-origin")
        || msg_lower.contains("preflight request")
    {
        return DevToolsExplanation {
            category: DiagnosticCategory::CorsPolicy,
            title: "CORS (Cross-Origin Resource Sharing) Blocked".to_string(),
            summary: "The browser prevented a cross-origin request because the destination server did not grant permission via CORS headers.".to_string(),
            root_cause: "The target server is missing 'Access-Control-Allow-Origin' matching this origin, or the preflight OPTIONS request returned a non-2xx status code.".to_string(),
            suggested_fix: "Configure the server to respond to OPTIONS preflight and attach 'Access-Control-Allow-Origin: <origin>' (or '*' for public APIs), or use a local dev proxy.".to_string(),
            security_implications: Some("CORS is a browser security mechanism protecting user sessions on cross-origin origins from unauthorized reads.".to_string()),
            redacted_sample,
        };
    }

    // 2. Content Security Policy (CSP)
    if msg_lower.contains("content security policy")
        || msg_lower.contains("violates the following content security policy directive")
        || msg_lower.contains("refused to load")
        || msg_lower.contains("refused to execute")
    {
        return DevToolsExplanation {
            category: DiagnosticCategory::ContentSecurityPolicy,
            title: "Content Security Policy (CSP) Violation".to_string(),
            summary: "The page's Content Security Policy blocked loading or execution of an external or inline script/resource.".to_string(),
            root_cause: "A resource or inline script eval was attempted that violates the server's 'script-src', 'connect-src', or 'default-src' directive.".to_string(),
            suggested_fix: "Add the required host to the corresponding CSP directive (e.g., connect-src https://api.example.com), or use cryptographic nonces rather than inline code.".to_string(),
            security_implications: Some("CSP mitigates Cross-Site Scripting (XSS) and malicious data injection attacks.".to_string()),
            redacted_sample,
        };
    }

    // 3. Mixed Content
    if msg_lower.contains("mixed content") || (msg_lower.contains("https") && msg_lower.contains("http://")) {
        return DevToolsExplanation {
            category: DiagnosticCategory::MixedContent,
            title: "Mixed Content Security Block".to_string(),
            summary: "An HTTPS page attempted to load insecure HTTP content.".to_string(),
            root_cause: "Modern browsers strictly block active HTTP content (scripts, fetch) and warn on passive HTTP content (images) when embedded in an HTTPS context.".to_string(),
            suggested_fix: "Upgrade all resource URLs and API endpoints to use 'https://' instead of 'http://'.".to_string(),
            security_implications: Some("Prevents man-in-the-middle attackers from sniffing or tampering with unencrypted resources on encrypted pages.".to_string()),
            redacted_sample,
        };
    }

    // 4. JavaScript Null / Undefined / TypeError
    if msg_lower.contains("cannot read properties of undefined")
        || msg_lower.contains("cannot read property")
        || msg_lower.contains("is not a function")
        || msg_lower.contains("typeerror")
        || msg_lower.contains("referenceerror")
    {
        let loc = if let (Some(l), Some(c)) = (diag.line, diag.column) {
            format!(" at line {l}:{c}")
        } else {
            String::new()
        };
        return DevToolsExplanation {
            category: DiagnosticCategory::JavaScriptException,
            title: format!("Unhandled JavaScript Exception{loc}"),
            summary: "A runtime error occurred when accessing a property or invoking a function on a null or undefined reference.".to_string(),
            root_cause: "The object being accessed was not initialized before this operation, or an asynchronous API response did not return the expected object shape.".to_string(),
            suggested_fix: "Use optional chaining (e.g. 'obj?.prop'), provide default fallback values, and verify async data resolution before rendering.".to_string(),
            security_implications: None,
            redacted_sample,
        };
    }

    // 5. Deprecation warning
    if msg_lower.contains("deprecated") || diag.level.to_lowercase() == "warning" {
        return DevToolsExplanation {
            category: DiagnosticCategory::DeprecationWarning,
            title: "Browser API Deprecation Warning".to_string(),
            summary: "A web platform API or feature used by this page is scheduled for removal.".to_string(),
            root_cause: "Usage of legacy DOM or network APIs that newer browser versions phase out in favor of standardized alternatives.".to_string(),
            suggested_fix: "Review browser console recommendations to adopt modern replacement APIs before the feature is disabled.".to_string(),
            security_implications: None,
            redacted_sample,
        };
    }

    // Generic fallback
    DevToolsExplanation {
        category: DiagnosticCategory::GenericError,
        title: "Console Diagnostic".to_string(),
        summary: "A diagnostic message was reported by the page runtime.".to_string(),
        root_cause: "Runtime evaluation error or unhandled promise rejection in client scripts.".to_string(),
        suggested_fix: "Inspect the stack trace and component lifecycle to trace the calling code.".to_string(),
        security_implications: None,
        redacted_sample,
    }
}

/// Explain a network error or HTTP failure with root cause and remediation (D-1).
pub fn explain_network_error(diag: &NetworkDiagnosticInput) -> DevToolsExplanation {
    let clean_url = redact_secrets(&diag.url);
    let sample = format!("{} {} -> {}", diag.method, clean_url, diag.status);

    match diag.status {
        401 => DevToolsExplanation {
            category: DiagnosticCategory::AuthenticationFailure,
            title: "HTTP 401: Unauthorized".to_string(),
            summary: "The request requires valid user authentication credentials.".to_string(),
            root_cause: "Missing, expired, or invalid authorization credentials (Bearer token or session cookie).".to_string(),
            suggested_fix: "Renew the authentication token or prompt the user to re-authenticate.".to_string(),
            security_implications: Some("Protects private endpoints from unauthenticated access.".to_string()),
            redacted_sample: sample,
        },
        403 => DevToolsExplanation {
            category: DiagnosticCategory::AuthorizationForbidden,
            title: "HTTP 403: Forbidden".to_string(),
            summary: "The server understood the request, but refuses to authorize it.".to_string(),
            root_cause: "The authenticated client lacks necessary permissions or role scopes, or a CSRF token verification failed.".to_string(),
            suggested_fix: "Verify user permission scopes or check for missing CSRF headers/tokens.".to_string(),
            security_implications: Some("Enforces role-based access control and anti-tampering guards.".to_string()),
            redacted_sample: sample,
        },
        404 => DevToolsExplanation {
            category: DiagnosticCategory::NotFound,
            title: "HTTP 404: Resource Not Found".to_string(),
            summary: "The requested endpoint or asset does not exist on the server.".to_string(),
            root_cause: "Incorrect route URL, missing query parameters, or static asset was not bundled in the build output.".to_string(),
            suggested_fix: "Verify URL route path spelling, ensure static files exist in the public directory, and check API router definitions.".to_string(),
            security_implications: None,
            redacted_sample: sample,
        },
        500 => DevToolsExplanation {
            category: DiagnosticCategory::ServerCrash,
            title: "HTTP 500: Internal Server Error".to_string(),
            summary: "The destination server encountered an unexpected condition that prevented it from fulfilling the request.".to_string(),
            root_cause: "An unhandled exception, database connection failure, or panic crashed the backend request handler.".to_string(),
            suggested_fix: "Inspect server-side application logs around the timestamp of the request to identify the unhandled panic/exception.".to_string(),
            security_implications: None,
            redacted_sample: sample,
        },
        502..=504 => DevToolsExplanation {
            category: DiagnosticCategory::ServerCrash,
            title: format!("HTTP {}: Bad Gateway / Gateway Timeout", diag.status),
            summary: "An intermediate proxy, CDN, or load balancer failed to receive a valid response from the upstream application.".to_string(),
            root_cause: "The application server process is down, overloaded, or timed out handling a long-running transaction.".to_string(),
            suggested_fix: "Check if the backend process is running, restart unhealthy pods, or increase reverse proxy timeout limits.".to_string(),
            security_implications: None,
            redacted_sample: sample,
        },
        0 => {
            let err = diag.error_text.as_deref().unwrap_or("net::ERR_FAILED").to_lowercase();
            if err.contains("connection_refused") {
                DevToolsExplanation {
                    category: DiagnosticCategory::NetworkTimeoutOrRefused,
                    title: "Network Error: Connection Refused".to_string(),
                    summary: "The browser could not establish a TCP connection to the host.".to_string(),
                    root_cause: "No server process is listening on the target port, or a local firewall is rejecting connection attempts.".to_string(),
                    suggested_fix: "Ensure your development server is running on the specified port (e.g. 'npm run dev' or 'cargo run').".to_string(),
                    security_implications: None,
                    redacted_sample: sample,
                }
            } else if err.contains("name_not_resolved") {
                DevToolsExplanation {
                    category: DiagnosticCategory::NetworkTimeoutOrRefused,
                    title: "Network Error: DNS Resolution Failed".to_string(),
                    summary: "The domain name could not be resolved to an IP address.".to_string(),
                    root_cause: "Non-existent domain name, typo in URL host, or DNS server connectivity loss.".to_string(),
                    suggested_fix: "Check domain spelling, verify internet connectivity, or check local hosts file mappings.".to_string(),
                    security_implications: None,
                    redacted_sample: sample,
                }
            } else {
                DevToolsExplanation {
                    category: DiagnosticCategory::NetworkTimeoutOrRefused,
                    title: "Network Connection Failed".to_string(),
                    summary: "The network request failed before receiving an HTTP response.".to_string(),
                    root_cause: format!("Low-level network failure: {}", diag.error_text.as_deref().unwrap_or("unknown")),
                    suggested_fix: "Inspect browser network tab, verify target URL, and check TLS/SSL certificate validity.".to_string(),
                    security_implications: None,
                    redacted_sample: sample,
                }
            }
        }
        _ => DevToolsExplanation {
            category: DiagnosticCategory::GenericError,
            title: format!("HTTP {} Response", diag.status),
            summary: format!("Server returned HTTP status code {}.", diag.status),
            root_cause: "Unexpected HTTP response code returned by backend server.".to_string(),
            suggested_fix: "Verify API documentation and request payload parameters.".to_string(),
            security_implications: None,
            redacted_sample: sample,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_cors_error_and_redacts_tokens() {
        let input = ConsoleDiagnosticInput {
            message: "Access to fetch at 'https://api.example.com/data?token=secret123' from origin 'http://localhost:3000' has been blocked by CORS policy: No 'Access-Control-Allow-Origin' header is present on the requested resource.".into(),
            level: "error".into(),
            source: Some("app.js".into()),
            line: Some(42),
            column: Some(10),
            stack_trace: None,
        };

        let expl = explain_console_error(&input);
        assert_eq!(expl.category, DiagnosticCategory::CorsPolicy);
        assert!(expl.title.contains("CORS"));
        assert!(expl.suggested_fix.contains("Access-Control-Allow-Origin"));
        assert!(!expl.redacted_sample.contains("secret123"));
        assert!(expl.redacted_sample.contains("[REDACTED]"));
    }

    #[test]
    fn explains_typeerror_and_preserves_line_info() {
        let input = ConsoleDiagnosticInput {
            message: "Uncaught TypeError: Cannot read properties of undefined (reading 'map')".into(),
            level: "error".into(),
            source: Some("bundle.js".into()),
            line: Some(128),
            column: Some(15),
            stack_trace: Some("at render (bundle.js:128:15)".into()),
        };

        let expl = explain_console_error(&input);
        assert_eq!(expl.category, DiagnosticCategory::JavaScriptException);
        assert!(expl.title.contains("line 128:15"));
        assert!(expl.suggested_fix.contains("optional chaining"));
    }

    #[test]
    fn explains_network_connection_refused() {
        let input = NetworkDiagnosticInput {
            url: "http://localhost:8080/api/users".into(),
            method: "GET".into(),
            status: 0,
            status_text: None,
            error_text: Some("net::ERR_CONNECTION_REFUSED".into()),
            headers: None,
        };

        let expl = explain_network_error(&input);
        assert_eq!(expl.category, DiagnosticCategory::NetworkTimeoutOrRefused);
        assert!(expl.title.contains("Connection Refused"));
        assert!(expl.suggested_fix.contains("development server"));
    }

    #[test]
    fn redacts_sensitive_headers() {
        let headers = vec![
            ("Authorization".to_string(), "Bearer secret-token-xyz".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Cookie".to_string(), "session=abc".to_string()),
        ];
        let redacted = redact_headers(&headers);
        assert_eq!(redacted[0].1, "[REDACTED]");
        assert_eq!(redacted[1].1, "application/json");
        assert_eq!(redacted[2].1, "[REDACTED]");
    }
}
