# Feature: scan_for_secrets - Security Scanning

**ID:** PHASE3-033  
**Priority:** 🔥🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 3 (Intelligence & Security)

---

## 📋 Описание

Сканирование кода на утечки секретов: API keys, passwords, tokens, private keys, database credentials. Проверка files + git history.

### Проблема

```
Developer: случайно закоммитил AWS key
→ Security breach!

Code review: не заметили leaked credentials
→ Production vulnerability
```

### Решение

```typescript
const leaks = await gofer.scan_for_secrets();

// Returns:
// ⚠️ CRITICAL: AWS Access Key in config/deploy.sh:12
// ⚠️ HIGH: Database password in .env.example:5
// ⚠️ MEDIUM: Private SSH key in backup/old_key
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Find: API keys, passwords, tokens, private keys
- ✅ Scan files + git history
- ✅ Multiple secret patterns (AWS, Stripe, GitHub, etc.)
- ✅ Severity assessment

### Non-Goals
- ❌ Не automatic secret rotation
- ❌ Не secret management (use vault)

---

## 🔧 API Specification

```json
{
  "name": "scan_for_secrets",
  "description": "Сканировать код на утечки секретов",
  "inputSchema": {
    "type": "object",
    "properties": {
      "scan_history": {
        "type": "boolean",
        "default": true,
        "description": "Сканировать git history"
      },
      "severity_filter": {
        "type": "string",
        "enum": ["critical", "high", "medium", "low", "all"],
        "default": "all"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct SecretLeak {
    pub secret_type: SecretType,
    pub file: String,
    pub line: u32,
    pub snippet: String,  // redacted
    pub severity: Severity,
    pub in_git_history: bool,
    pub first_seen: Option<DateTime<Utc>>,
    pub recommendation: String,
}

#[derive(Serialize)]
pub enum SecretType {
    AwsAccessKey,
    AwsSecretKey,
    StripeApiKey,
    GitHubToken,
    DatabasePassword,
    PrivateKey,
    GenericApiKey,
}

#[derive(Serialize)]
pub enum Severity {
    Critical,  // Active production secrets
    High,      // Valid secrets, not in production
    Medium,    // Example/test secrets
    Low,       // False positive likely
}
```

---

## 💻 Implementation

```rust
pub async fn scan_for_secrets(
    scan_history: bool
) -> Result<Vec<SecretLeak>> {
    let mut leaks = Vec::new();
    
    // 1. Scan current files
    let file_leaks = scan_workspace_files().await?;
    leaks.extend(file_leaks);
    
    // 2. Scan git history if requested
    if scan_history {
        let history_leaks = scan_git_history().await?;
        leaks.extend(history_leaks);
    }
    
    // 3. Deduplicate and assess severity
    leaks = deduplicate_and_assess(leaks).await?;
    
    Ok(leaks)
}

async fn scan_workspace_files() -> Result<Vec<SecretLeak>> {
    let patterns = load_secret_patterns();
    let mut leaks = Vec::new();
    
    // Glob all files
    let files = glob("**/*")?;
    
    for file in files {
        let content = fs::read_to_string(&file)?;
        
        for (line_num, line) in content.lines().enumerate() {
            for pattern in &patterns {
                if let Some(matched) = pattern.regex.find(line) {
                    leaks.push(SecretLeak {
                        secret_type: pattern.secret_type.clone(),
                        file: file.display().to_string(),
                        line: line_num as u32 + 1,
                        snippet: redact_secret(line, matched.start(), matched.end()),
                        severity: assess_severity(&pattern, &file),
                        in_git_history: false,
                        first_seen: None,
                        recommendation: generate_recommendation(&pattern),
                    });
                }
            }
        }
    }
    
    Ok(leaks)
}

async fn scan_git_history() -> Result<Vec<SecretLeak>> {
    // Use gitleaks or similar
    let output = Command::new("gitleaks")
        .args(&["detect", "--no-git", "--report-format", "json"])
        .output()?;
    
    let leaks: Vec<GitLeaksResult> = serde_json::from_slice(&output.stdout)?;
    
    // Convert to our format
    Ok(leaks.into_iter().map(convert_gitleaks_result).collect())
}

fn load_secret_patterns() -> Vec<SecretPattern> {
    vec![
        SecretPattern {
            name: "AWS Access Key",
            secret_type: SecretType::AwsAccessKey,
            regex: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
        },
        SecretPattern {
            name: "AWS Secret Key",
            secret_type: SecretType::AwsSecretKey,
            regex: Regex::new(r"(?i)aws(.{0,20})?['\"][0-9a-zA-Z/+]{40}['\"]").unwrap(),
        },
        SecretPattern {
            name: "Stripe API Key",
            secret_type: SecretType::StripeApiKey,
            regex: Regex::new(r"sk_live_[0-9a-zA-Z]{24}").unwrap(),
        },
        SecretPattern {
            name: "GitHub Token",
            secret_type: SecretType::GitHubToken,
            regex: Regex::new(r"ghp_[0-9a-zA-Z]{36}").unwrap(),
        },
        // ... more patterns
    ]
}

fn assess_severity(pattern: &SecretPattern, file: &Path) -> Severity {
    // Check if in production config
    if file.to_str().unwrap().contains("production") {
        return Severity::Critical;
    }
    
    // Check if example file
    if file.to_str().unwrap().contains("example") 
        || file.to_str().unwrap().contains("test") {
        return Severity::Medium;
    }
    
    // Default: High
    Severity::High
}

fn redact_secret(line: &str, start: usize, end: usize) -> String {
    let mut redacted = line.to_string();
    let secret_len = end - start;
    let show_chars = (secret_len / 4).min(4);
    
    let prefix = &line[start..start+show_chars];
    let redacted_part = "*".repeat(secret_len - show_chars);
    
    format!("{}...{}", prefix, redacted_part)
}
```

---

## 📈 Success Metrics

- ✅ Finds 95%+ real secrets
- ✅ False positive rate < 10%
- ⏱️ Scan time < 30s для 1000 files

---

## ✅ Acceptance Criteria

- [ ] Scans current files
- [ ] Scans git history
- [ ] Multiple secret types detected
- [ ] Severity assessment accurate
- [ ] < 10% false positives
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16

**CRITICAL:** Запускать регулярно! Security breach prevention.
