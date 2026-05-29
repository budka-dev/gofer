# Feature: check_dependencies - Vulnerability Scanning

**ID:** PHASE3-034  
**Priority:** 🔥🔥🔥🔥 Critical  
**Effort:** 3 дня  
**Status:** Not Started  
**Phase:** 3 (Intelligence & Security)

---

## 📋 Описание

Сканирование dependencies на известные уязвимости (CVE). Интеграция с cargo-audit, npm audit, safety для проверки всех пакетов.

### Проблема

```
Production dependency: log4j 2.14.0
→ CVE-2021-44228 (Log4Shell) - CRITICAL

Developer: не знает об уязвимости
→ Security breach potential
```

### Решение

```typescript
const vulns = await gofer.check_dependencies();

// Returns:
// ⚠️ CRITICAL: log4j 2.14.0 - CVE-2021-44228 (RCE)
//   Fix: Upgrade to 2.17.0+
// ⚠️ HIGH: axios 0.19.0 - CVE-2020-28168
//   Fix: Upgrade to 0.21.1+
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Scan Rust, JavaScript, Python dependencies
- ✅ CVE database integration
- ✅ Severity + fix availability
- ✅ Automated scanning

### Non-Goals
- ❌ Не automatic patching
- ❌ Не license compliance (separate tool)

---

## 🔧 API Specification

```json
{
  "name": "check_dependencies",
  "description": "Проверить dependencies на CVE",
  "inputSchema": {
    "type": "object",
    "properties": {
      "ecosystem": {
        "type": "string",
        "enum": ["all", "cargo", "npm", "pip"],
        "default": "all"
      },
      "severity_filter": {
        "type": "string",
        "enum": ["critical", "high", "medium", "low", "all"],
        "default": "high"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct Vulnerability {
    pub package: String,
    pub version: String,
    pub cve_id: String,
    pub severity: Severity,
    pub description: String,
    pub fix_available: bool,
    pub fixed_version: Option<String>,
    pub published_date: DateTime<Utc>,
}

#[derive(Serialize)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}
```

---

## 💻 Implementation

```rust
pub async fn check_dependencies(
    ecosystem: Ecosystem
) -> Result<Vec<Vulnerability>> {
    let mut vulnerabilities = Vec::new();
    
    match ecosystem {
        Ecosystem::All | Ecosystem::Cargo => {
            let cargo_vulns = check_cargo_deps().await?;
            vulnerabilities.extend(cargo_vulns);
        }
        Ecosystem::All | Ecosystem::Npm => {
            let npm_vulns = check_npm_deps().await?;
            vulnerabilities.extend(npm_vulns);
        }
        Ecosystem::All | Ecosystem::Pip => {
            let pip_vulns = check_pip_deps().await?;
            vulnerabilities.extend(pip_vulns);
        }
    }
    
    Ok(vulnerabilities)
}

async fn check_cargo_deps() -> Result<Vec<Vulnerability>> {
    // Run cargo-audit
    let output = Command::new("cargo")
        .args(&["audit", "--json"])
        .output()?;
    
    let report: CargoAuditReport = serde_json::from_slice(&output.stdout)?;
    
    let mut vulns = Vec::new();
    
    for vuln in report.vulnerabilities.list {
        vulns.push(Vulnerability {
            package: vuln.package.name,
            version: vuln.package.version,
            cve_id: vuln.advisory.id,
            severity: parse_severity(&vuln.advisory.cvss),
            description: vuln.advisory.description,
            fix_available: vuln.versions.patched.is_some(),
            fixed_version: vuln.versions.patched.first().cloned(),
            published_date: vuln.advisory.date,
        });
    }
    
    Ok(vulns)
}

async fn check_npm_deps() -> Result<Vec<Vulnerability>> {
    // Run npm audit
    let output = Command::new("npm")
        .args(&["audit", "--json"])
        .output()?;
    
    let report: NpmAuditReport = serde_json::from_slice(&output.stdout)?;
    
    // Parse npm audit format
    // ...
    
    Ok(vec![])
}

async fn check_pip_deps() -> Result<Vec<Vulnerability>> {
    // Run safety check
    let output = Command::new("safety")
        .args(&["check", "--json"])
        .output()?;
    
    // Parse safety output
    // ...
    
    Ok(vec![])
}

fn parse_severity(cvss: &str) -> Severity {
    // Parse CVSS score
    let score: f32 = cvss.parse().unwrap_or(0.0);
    
    match score {
        s if s >= 9.0 => Severity::Critical,
        s if s >= 7.0 => Severity::High,
        s if s >= 4.0 => Severity::Medium,
        _ => Severity::Low,
    }
}
```

---

## 📈 Success Metrics

- ✅ Finds 100% known CVEs
- ✅ Fix recommendations accurate
- ⏱️ Scan time < 10s

---

## ✅ Acceptance Criteria

- [ ] Cargo dependencies scanned
- [ ] NPM dependencies scanned
- [ ] Python dependencies scanned
- [ ] CVE database up-to-date
- [ ] Fix versions suggested
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16

**CRITICAL:** Run daily! Security vulnerabilities evolve.
